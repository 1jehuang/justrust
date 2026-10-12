// justrust hot patch loader (src/hotpatch.rs, FINDINGS 42).
//
// Linked into a hot-patchable test binary as a DT_NEEDED library (absolute
// path, next to the binary in `<exe>.jrhot/`), so its constructor runs
// before any initializer of the executable, whoever starts the binary:
// cargo, justrust's test pool, a test that re-executes itself, a user.
//
// If `<exe>.jrhot/map` was written for exactly this executable (inode, size,
// mtime), it maps the patch image (the recompiled codegen units, linked
// against the base's addresses), applies its relocations, turns the entry of
// every replaced function in the base into `jmp rel32` to its new copy, and
// registers the image's unwind tables. Without a map it does nothing: the
// base binary is current.
//
// Addresses in the map assume the executable at PIE_BASE. Everything moves
// by `bias - PIE_BASE` with ASLR on; the image goes PATCH_SPAN below the
// executable (within jmp rel32 reach). If that range is taken, the process
// re-executes itself once with ASLR off, where it is always free.
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
#include <sys/personality.h>
#include <sys/stat.h>
#include <unistd.h>

extern void __register_frame(void *);

static void die(const char *msg) {
    fprintf(stderr,
            "justrust hotpatch: %s (rerun with JUSTRUST_HOTPATCH=0 to relink normally)\n", msg);
    _exit(97);
}

static int first_bias(struct dl_phdr_info *info, size_t size, void *data) {
    (void)size;
    *(uintptr_t *)data = info->dlpi_addr;
    return 1; // the first entry is the main executable
}

static struct { uintptr_t start, len; int prot; } segs[16];
static int nsegs;

// 0: mapped, 1: address range taken (caller may retry without ASLR).
static int map_image(const char *path, intptr_t delta) {
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) die("cannot open patch image");
    Elf64_Ehdr eh;
    if (pread(fd, &eh, sizeof eh, 0) != sizeof eh) die("short patch image");
    for (int i = 0; i < eh.e_phnum; i++) {
        Elf64_Phdr ph;
        if (pread(fd, &ph, sizeof ph, eh.e_phoff + (off_t)i * eh.e_phentsize) != sizeof ph)
            die("bad program header");
        if (ph.p_type != PT_LOAD || ph.p_memsz == 0) continue;
        uintptr_t va = ph.p_vaddr + delta;
        uintptr_t start = va & ~(uintptr_t)0xfff;
        uintptr_t end = (va + ph.p_memsz + 0xfff) & ~(uintptr_t)0xfff;
        void *want = (void *)start;
        void *got = mmap(want, end - start, PROT_READ | PROT_WRITE,
                         MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
        if (got != want) {
            if (got != MAP_FAILED) munmap(got, end - start);
            for (int j = 0; j < nsegs; j++) munmap((void *)segs[j].start, segs[j].len);
            nsegs = 0;
            close(fd);
            return 1;
        }
        if (ph.p_filesz && pread(fd, (void *)va, ph.p_filesz, ph.p_offset) != (ssize_t)ph.p_filesz)
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
    return 0;
}

static void reexec_without_aslr(char **argv) {
    if (getenv("JUSTRUST_HOTPATCH_REEXEC")) die("patch address range is taken");
    setenv("JUSTRUST_HOTPATCH_REEXEC", "1", 1);
    personality(ADDR_NO_RANDOMIZE);
    extern char **environ;
    execve("/proc/self/exe", argv, environ);
    die("re-exec without ASLR failed");
}

__attribute__((constructor)) static void jr_hotpatch(int argc, char **argv, char **envp) {
    (void)argc;
    (void)envp;
    char exe[4096];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 16);
    if (n <= 0) return;
    exe[n] = 0;
    char map[4200];
    snprintf(map, sizeof map, "%s.jrhot/map", exe);
    FILE *f = fopen(map, "re");
    if (!f) return; // no patch: the base is current
    unsetenv("JUSTRUST_HOTPATCH_REEXEC");
    char line[512], image[4096] = {0};
    unsigned long long ino = 0, size = 0, ms = 0, mns = 0, base = 0, eh = 0;
    if (!fgets(line, sizeof line, f) ||
        sscanf(line, "X %llu %llu %llu %llu", &ino, &size, &ms, &mns) != 4) {
        fclose(f);
        return;
    }
    struct stat st;
    if (stat("/proc/self/exe", &st) || st.st_ino != ino || (unsigned long long)st.st_size != size ||
        (unsigned long long)st.st_mtim.tv_sec != ms || (unsigned long long)st.st_mtim.tv_nsec != mns) {
        // A map for another binary at this path (relinked since): stale.
        fclose(f);
        return;
    }
    if (!fgets(line, sizeof line, f) || sscanf(line, "B %llx", &base) != 1) die("bad map (B)");
    if (!fgets(line, sizeof line, f) || sscanf(line, "I %4095s", image) != 1) die("bad map (I)");
    if (!fgets(line, sizeof line, f) || sscanf(line, "E %llx", &eh) != 1) die("bad map (E)");
    uintptr_t bias = 0;
    dl_iterate_phdr(first_bias, &bias);
    intptr_t delta = (intptr_t)(bias - base);
    if (map_image(image, delta)) {
        fclose(f);
        reexec_without_aslr(argv);
    }

    uintptr_t page = 0;
    unsigned long long from, to;
    long pagesz = sysconf(_SC_PAGESIZE);
    char name[400], kind;
    unsigned long long add;
    while (fgets(line, sizeof line, f)) {
        if (sscanf(line, "A %llx %llx", &from, &to) == 2) {
            *(uint64_t *)(uintptr_t)(from + delta) = to + delta;
            continue;
        }
        if (sscanf(line, "%c %llx %llx %399s", &kind, &from, &add, name) == 4 &&
            (kind == 'D' || kind == 'W')) {
            void *sym = dlsym(RTLD_DEFAULT, name);
            if (!sym && kind == 'D') {
                fprintf(stderr, "justrust hotpatch: unresolved %s\n", name);
                _exit(97);
            }
            *(uint64_t *)(uintptr_t)(from + delta) = sym ? (uint64_t)(uintptr_t)sym + add : 0;
            continue;
        }
        if (sscanf(line, "T %llx %llx", &from, &to) != 2) continue;
        from += delta;
        to += delta;
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
    if (eh) __register_frame((void *)(uintptr_t)(eh + delta));
    fclose(f);
}
