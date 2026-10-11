// justrust hot patch loader (prototype, bench/hotpatch/README.md).
//
// LD_PRELOADed into a test binary by `hotrun`. Before the executable's own
// initializers run, it maps the patch image (the recompiled codegen units,
// linked at a fixed address next to the base binary), registers its unwind
// tables, and turns the entry of every replaced function in the base binary
// into `jmp rel32` to the new copy. Nothing happens unless JR_HOTPATCH names
// a map file written for exactly this executable (inode, size, mtime).
#define _GNU_SOURCE
#include <dlfcn.h>
#include <elf.h>
#include <fcntl.h>
#include <link.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

extern void __register_frame(void *);

static void die(const char *msg) {
    fprintf(stderr, "justrust hotpatch: %s\n", msg);
    _exit(97);
}

static int first_bias(struct dl_phdr_info *info, size_t size, void *data) {
    (void)size;
    *(uintptr_t *)data = info->dlpi_addr;
    return 1; // the first entry is the main executable
}

// Segments of the patch image, made final after relocation.
static struct { uintptr_t start, len; int prot; } segs[16];
static int nsegs;

static void map_image(const char *path) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) die("cannot open patch image");
    Elf64_Ehdr eh;
    if (pread(fd, &eh, sizeof eh, 0) != sizeof eh) die("short patch image");
    for (int i = 0; i < eh.e_phnum; i++) {
        Elf64_Phdr ph;
        if (pread(fd, &ph, sizeof ph, eh.e_phoff + (off_t)i * eh.e_phentsize) != sizeof ph)
            die("bad program header");
        if (ph.p_type != PT_LOAD || ph.p_memsz == 0) continue;
        uintptr_t start = ph.p_vaddr & ~(uintptr_t)0xfff;
        uintptr_t end = (ph.p_vaddr + ph.p_memsz + 0xfff) & ~(uintptr_t)0xfff;
        void *want = (void *)start;
        void *got = mmap(want, end - start, PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
        if (got != want) die("patch address range is taken");
        if (ph.p_filesz &&
            pread(fd, (void *)ph.p_vaddr, ph.p_filesz, ph.p_offset) != (ssize_t)ph.p_filesz)
            die("short read of patch segment");
        int prot = ((ph.p_flags & PF_R) ? PROT_READ : 0) | ((ph.p_flags & PF_W) ? PROT_WRITE : 0) |
                   ((ph.p_flags & PF_X) ? PROT_EXEC : 0);
        if (nsegs == 16) die("too many patch segments");
        segs[nsegs].start = start;
        segs[nsegs].len = end - start;
        segs[nsegs].prot = prot;
        nsegs++;
    }
    close(fd);
}

__attribute__((constructor)) static void jr_hotpatch(void) {
    const char *map = getenv("JR_HOTPATCH");
    if (!map || !*map) return;
    FILE *f = fopen(map, "re");
    if (!f) return;
    char line[512], image[4096] = {0};
    unsigned long long ino = 0, size = 0, mtime = 0, base = 0, eh = 0;
    // Header: exe identity, expected load address, image, eh_frame.
    if (!fgets(line, sizeof line, f) ||
        sscanf(line, "X %llu %llu %llu", &ino, &size, &mtime) != 3) {
        fclose(f);
        return;
    }
    struct stat st;
    // Another program started by a test inherits LD_PRELOAD: leave it alone.
    if (stat("/proc/self/exe", &st) || st.st_ino != ino || (unsigned long long)st.st_size != size ||
        (unsigned long long)st.st_mtime != mtime) {
        fclose(f);
        return;
    }
    if (!fgets(line, sizeof line, f) || sscanf(line, "B %llx", &base) != 1) die("bad map (B)");
    if (!fgets(line, sizeof line, f) || sscanf(line, "I %4095s", image) != 1) die("bad map (I)");
    if (!fgets(line, sizeof line, f) || sscanf(line, "E %llx", &eh) != 1) die("bad map (E)");
    uintptr_t bias = 0;
    dl_iterate_phdr(first_bias, &bias);
    if (bias != base) die("executable not at the expected address (ASLR on?)");
    map_image(image);

    uintptr_t page = 0;
    unsigned long long from, to;
    long pagesz = sysconf(_SC_PAGESIZE);
    char name[400], kind;
    unsigned long long add;
    while (fgets(line, sizeof line, f)) {
        if (sscanf(line, "A %llx %llx", &from, &to) == 2) {
            // A relocation of the patch image, value already final.
            *(uint64_t *)(uintptr_t)from = to;
            continue;
        }
        if (sscanf(line, "%c %llx %llx %399s", &kind, &from, &add, name) == 4 &&
            (kind == 'D' || kind == 'W')) {
            // A shared-library import of the patch (malloc, pthread_*, ...).
            void *sym = dlsym(RTLD_DEFAULT, name);
            if (!sym && kind == 'D') {
                fprintf(stderr, "justrust hotpatch: unresolved %s\n", name);
                _exit(97);
            }
            *(uint64_t *)(uintptr_t)from = sym ? (uint64_t)(uintptr_t)sym + add : 0;
            continue;
        }
        if (sscanf(line, "T %llx %llx", &from, &to) != 2) continue;
        uintptr_t p0 = from & ~(uintptr_t)(pagesz - 1);
        if (p0 != page) {
            if (page) mprotect((void *)page, 2 * pagesz, PROT_READ | PROT_EXEC);
            page = p0;
            if (mprotect((void *)page, 2 * pagesz, PROT_READ | PROT_WRITE | PROT_EXEC))
                die("mprotect base text");
        }
        int64_t rel = (int64_t)to - (int64_t)(from + 5);
        if (rel > INT32_MAX || rel < INT32_MIN) die("patch out of jmp range");
        unsigned char *c = (unsigned char *)(uintptr_t)from;
        int32_t r32 = (int32_t)rel;
        c[0] = 0xe9;
        memcpy(c + 1, &r32, 4);
    }
    if (page) mprotect((void *)page, 2 * pagesz, PROT_READ | PROT_EXEC);
    for (int i = 0; i < nsegs; i++)
        if (mprotect((void *)segs[i].start, segs[i].len, segs[i].prot)) die("mprotect patch segment");
    // Unwinding through patched code (panics in tests) needs its FDEs.
    if (eh) __register_frame((void *)(uintptr_t)eh);
    fclose(f);
}
