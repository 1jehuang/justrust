// The recorder and the private writes of the sandbox on macOS (`sandbox/macos.rs`): a library
// that every process of the sandbox loads through `DYLD_INSERT_LIBRARIES`, which replaces the C
// library's calls that name a path, run a program or reach the network (dyld's
// `__DATA,__interpose`). It
//
// - notes each path the process opens, looks at, lists, runs or changes, each write and each
//   address the sandbox refused, and each program it ran, in the run's record file;
// - keeps the writes under the overlays to the run: a changed file is first cloned into the
//   run's upper directory (`clonefile`, which on APFS copies no data), and from then on the
//   path is answered from there; a removed one is hidden by a whiteout there;
// - answers the system's temporary directories from empty ones of the run's own;
// - names the test of the calling thread from the harness's marker
//   (`access("/.ra-jit-test/<name>")`).
//
// Seatbelt (`sandbox_init`) is what enforces: everything here only redirects and records, so
// a process it is not loaded in (a program macOS protects drops `DYLD_*`) still can't write
// outside the run or reach the network. Its configuration is the file `config` in the
// directory `__RA_JIT_SANDBOX` names, written by `macos.rs`:
//
//     O\t<index>\t<directory>\t<upper>   an overlay (one line for each way to spell it)
//     B\t<directory>\t<target>           a directory answered from `target`
//     W\t<directory>                     a directory written for real
//     L                                  record (W, N and X lines, and E and H for X)
//     R                                  record reads (R lines) too
//
// Records are lines appended to `record` in that directory with one write each, which `macos.rs`
// reads when the run ends:
//
//     R\t<test>\t<path>      a path named
//     W\t<test>\t<path>      a write the sandbox refused
//     N\t<test>\t<address>   an address the sandbox refused
//     X\t<test>\t<why>       the record is not whole
//     E\t<nonce>\t<test>\t<path>   a program run, with the nonce its process is given
//     H\t<nonce>             a process that loaded this library, by the nonce it was given
//     F\t<nonce>             a program that did not run after all
//
// An E with no H is a program that ran without this library.

#define _DARWIN_C_SOURCE
#include <copyfile.h>
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <mach-o/dyld.h>
#include <netinet/in.h>
#include <os/lock.h>
#include <pthread.h>
#include <spawn.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/attr.h>
#include <sys/clonefile.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/un.h>
#include <sys/xattr.h>
#include <unistd.h>

extern char **environ;

#define INTERPOSE(mine, theirs)                                                            \
    __attribute__((used)) static const struct {                                           \
        const void *replacement;                                                           \
        const void *replacee;                                                              \
    } interpose_##mine __attribute__((section("__DATA,__interpose"))) = {                \
        (const void *)(mine), (const void *)(theirs)};

// The variants the C library calls itself (`fopen`, `opendir`), which plain `open` misses.
extern int open_nocancel(const char *, int, ...) __asm("_open$NOCANCEL");
extern int openat_nocancel(int, const char *, int, ...) __asm("_openat$NOCANCEL");
extern int __open(const char *, int, int);
extern int __open_nocancel(const char *, int, int);
extern int __openat(int, const char *, int, int);
extern int __openat_nocancel(int, const char *, int, int);
extern int fcntl_nocancel(int, int, ...) __asm("_fcntl$NOCANCEL");
extern int connect_nocancel(int, const struct sockaddr *, socklen_t) __asm("_connect$NOCANCEL");
extern ssize_t sendto_nocancel(int, const void *, size_t, int, const struct sockaddr *, socklen_t)
    __asm("_sendto$NOCANCEL");
extern ssize_t sendmsg_nocancel(int, const struct msghdr *, int) __asm("_sendmsg$NOCANCEL");
extern int renameatx_np(int, const char *, int, const char *, unsigned int);
extern int renamex_np(const char *, const char *, unsigned int);
extern int getattrlistat(int, const char *, void *, void *, size_t, unsigned long);

#define MARK "/.ra-jit-test/"
#define WHITEOUT "\x01ra-jit-whiteout\x01"
#define MAX_ROOTS 64
#define MAX_OVERLAYS 32

enum kind { NONE, OVERLAY, BIND, WRITABLE, SCRATCH };

struct root {
    char *path;
    size_t len;
    enum kind kind;
    int index;      // the overlay's
    char *target;   // the overlay's upper, or what a bound directory is answered from
    size_t target_len;
};

// What every process of the run sees of the others: whether an overlay has anything in its
// upper directory yet, and whether there is any whiteout. Until then a read needs no look
// at the upper directories.
struct shared {
    _Atomic uint32_t written[MAX_OVERLAYS];
    _Atomic uint32_t whiteouts;
    _Atomic uint32_t lists;
};

static struct root roots[MAX_ROOTS];
static int nroots;
static char scratch[PATH_MAX];
static size_t scratch_len;
static char lists[PATH_MAX];
static size_t lists_len;
static struct shared *shared;
static int record_fd = -1;
static int record_reads;
// Whether the library is set up; until then (dyld and libSystem starting) calls go through.
static int alive;
static char env_insert[PATH_MAX + 32];
static char self[PATH_MAX];         // this library's path
static char env_dir[PATH_MAX + 32];

static __thread int busy;           // inside a call here: the C library's calls go through
static __thread char test[512];     // the test the thread runs, from the marker

// ---------------------------------------------------------------------------------------
// Records.

static os_unfair_lock seen_lock = OS_UNFAIR_LOCK_INIT;
static uint64_t *seen;
static size_t seen_size, seen_count;

static uint64_t hash(const char *a, const char *b) {
    uint64_t h = 0xcbf29ce484222325ull;
    for (; *a; a++) h = (h ^ (uint8_t)*a) * 0x100000001b3ull;
    h = (h ^ 0xff) * 0x100000001b3ull;
    for (; *b; b++) h = (h ^ (uint8_t)*b) * 0x100000001b3ull;
    return h | 1;
}

// Whether the line is new to this process; the merge removes what several processes repeat.
static int first_time(const char *a, const char *b) {
    uint64_t h = hash(a, b);
    int fresh = 0;
    os_unfair_lock_lock(&seen_lock);
    if (seen_count * 2 >= seen_size) {
        size_t size = seen_size ? seen_size * 2 : 1 << 14;
        uint64_t *grown = mmap(NULL, size * 8, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if (grown == MAP_FAILED) {
            os_unfair_lock_unlock(&seen_lock);
            return 1;
        }
        for (size_t i = 0; i < seen_size; i++) {
            if (!seen[i]) continue;
            size_t at = seen[i] & (size - 1);
            while (grown[at]) at = (at + 1) & (size - 1);
            grown[at] = seen[i];
        }
        if (seen) munmap(seen, seen_size * 8);
        seen = grown;
        seen_size = size;
    }
    size_t at = h & (seen_size - 1);
    while (seen[at] && seen[at] != h) at = (at + 1) & (seen_size - 1);
    if (!seen[at]) {
        seen[at] = h;
        seen_count++;
        fresh = 1;
    }
    os_unfair_lock_unlock(&seen_lock);
    return fresh;
}

static void put(char *line, size_t *at, size_t cap, const char *text) {
    for (; *text && *at + 1 < cap; text++) {
        // A tab or a newline in a name would break the line apart.
        line[(*at)++] = (*text == '\t' || *text == '\n') ? ' ' : *text;
    }
}

static void emit(const char *kind, const char *a, const char *b, const char *c) {
    if (record_fd < 0) return;
    char line[3 * PATH_MAX];
    size_t at = 0;
    put(line, &at, sizeof line, kind);
    for (const char *part[] = {a, b, c}, **it = part; it < part + 3; it++) {
        if (!*it) break;
        line[at++] = '\t';
        put(line, &at, sizeof line - 1, *it);
    }
    line[at++] = '\n';
    int saved = errno;
    write(record_fd, line, at);
    errno = saved;
}

static void note(char kind, const char *path) {
    if (record_fd < 0) return;
    // A tab or a line break would end the field: the record says it is not whole instead,
    // as `Recorder` in `sandbox.rs` does.
    if (strpbrk(path, "\t\n")) {
        emit("X", test, "a path with a tab or a line break in it", NULL);
        return;
    }
    char k[2] = {kind, 0};
    if (kind == 'R' && !first_time(test, path)) return;
    emit(k, test, path, NULL);
}

// ---------------------------------------------------------------------------------------
// Paths.

static os_unfair_lock cwd_lock = OS_UNFAIR_LOCK_INIT;
static char cwd[PATH_MAX];
static int cwd_known;

static void reverse(char *path);

// `path` with `.`, `..` and repeated slashes taken out by their spelling, in place.
static void tidy(char *path) {
    char *out = path, *in = path;
    while (*in) {
        while (*in == '/') in++;
        if (!*in) break;
        char *end = in;
        while (*end && *end != '/') end++;
        size_t len = end - in;
        if (len == 1 && in[0] == '.') {
        } else if (len == 2 && in[0] == '.' && in[1] == '.') {
            while (out > path && *--out != '/') {}
        } else {
            *out++ = '/';
            memmove(out, in, len);
            out += len;
        }
        in = end;
    }
    if (out == path) *out++ = '/';
    *out = 0;
}

static int join(char *out, const char *base, const char *rest) {
    size_t a = strlen(base), b = strlen(rest);
    if (a + b + 2 > PATH_MAX) return -1;
    memcpy(out, base, a);
    out[a] = '/';
    memcpy(out + a + 1, rest, b + 1);
    return 0;
}

// The whole path `path` names from `dirfd`, as the program sees it. -1 when it can't be told.
static int absolute(int dirfd, const char *path, char *out) {
    if (!path || !*path) return -1;
    if (path[0] == '/') {
        if (strlen(path) >= PATH_MAX) return -1;
        strcpy(out, path);
    } else if (dirfd == AT_FDCWD) {
        char base[PATH_MAX];
        os_unfair_lock_lock(&cwd_lock);
        if (!cwd_known && getcwd(cwd, sizeof cwd)) {
            reverse(cwd);
            cwd_known = 1;
        }
        int known = cwd_known;
        if (known) strcpy(base, cwd);
        os_unfair_lock_unlock(&cwd_lock);
        if (!known || join(out, base, path) < 0) return -1;
    } else {
        char base[PATH_MAX];
        if (fcntl(dirfd, F_GETPATH, base) < 0) return -1;
        reverse(base);
        if (join(out, base, path) < 0) return -1;
    }
    tidy(out);
    return 0;
}

static int under(const char *path, const char *dir, size_t len) {
    return strncmp(path, dir, len) == 0 && (path[len] == '/' || path[len] == 0 || len == 1);
}

// The root `path` is under, the one named longest, and the rest of it after the root.
static struct root *where(const char *path, const char **rest) {
    for (int i = 0; i < nroots; i++) {
        if (under(path, roots[i].path, roots[i].len)) {
            *rest = path + roots[i].len;
            return &roots[i];
        }
    }
    return NULL;
}

// A path in the run's own directories as the program knows it: an upper's or a bound
// directory's under the directory it stands for, a listing's under what it lists.
static void reverse(char *path) {
    if (!scratch_len || !under(path, scratch, scratch_len)) return;
    char out[PATH_MAX];
    if (under(path, lists, lists_len)) {
        const char *rest = path + lists_len;
        rest = strchr(rest + 1, '/');
        strcpy(path, rest ? rest : "/");
        return;
    }
    for (int i = 0; i < nroots; i++) {
        struct root *it = &roots[i];
        if ((it->kind == OVERLAY || it->kind == BIND) &&
            under(path, it->target, it->target_len)) {
            if (snprintf(out, sizeof out, "%s%s", it->path, path + it->target_len) < PATH_MAX)
                strcpy(path, out);
            return;
        }
    }
}

// ---------------------------------------------------------------------------------------
// Overlays.

enum how { READ, LIST, CHANGE, CREATE, CREATE_EMPTY };
enum answer { PASS, USE, FAIL };
enum state { MISSING, PRESENT, WHITED, NOT_DIR };

static int whiteout_at(const char *upper, const struct stat *st) {
    if (!S_ISLNK(st->st_mode) || st->st_size != (off_t)strlen(WHITEOUT)) return 0;
    char target[64];
    ssize_t n = readlink(upper, target, sizeof target);
    return n == (ssize_t)strlen(WHITEOUT) && memcmp(target, WHITEOUT, n) == 0;
}

static enum state state_of(const char *upper, struct stat *st) {
    if (lstat(upper, st) < 0) return errno == ENOTDIR ? NOT_DIR : MISSING;
    return whiteout_at(upper, st) ? WHITED : PRESENT;
}

static int whiteout(const char *upper) {
    if (symlink(WHITEOUT, upper) < 0) return -1;
    atomic_fetch_add(&shared->whiteouts, 1);
    return 0;
}

static void written(struct root *it) {
    if (!atomic_load_explicit(&shared->written[it->index], memory_order_relaxed))
        atomic_store(&shared->written[it->index], 1);
}

// Whether a directory above `rest` in the upper is a whiteout or not a directory, which
// hides what the lower has below it. Only looked at once there is some whiteout.
static int hidden(struct root *it, const char *rest, int *error) {
    if (!*rest) return 0;
    char upper[PATH_MAX];
    size_t len = it->target_len;
    memcpy(upper, it->target, len);
    for (const char *at = rest + 1; (at = strchr(at, '/')); at++) {
        size_t part = at - rest;
        if (len + part >= PATH_MAX) return 0;
        memcpy(upper + len, rest, part);
        upper[len + part] = 0;
        struct stat st;
        switch (state_of(upper, &st)) {
        case MISSING: return 0;
        case WHITED: *error = ENOENT; return 1;
        case NOT_DIR: *error = ENOTDIR; return 1;
        case PRESENT:
            if (!S_ISDIR(st.st_mode)) {
                *error = ENOTDIR;
                return 1;
            }
        }
    }
    return 0;
}

// Copies `lower` to `upper`: a clone where the volume can (APFS), else a copy; a directory
// as an empty one, its entries staying the lower's until they change; an empty file when its
// contents are about to be cut off anyway.
static int copy_up(const char *lower, const char *upper, const struct stat *st, int empty) {
    int r;
    if (S_ISDIR(st->st_mode)) {
        r = mkdir(upper, st->st_mode & 07777);
    } else if (empty && S_ISREG(st->st_mode)) {
        r = open(upper, O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, st->st_mode & 07777);
        if (r >= 0) r = close(r);
    } else {
        r = clonefile(lower, upper, CLONE_NOFOLLOW);
        if (r < 0 && errno != EEXIST)
            r = copyfile(lower, upper, NULL, COPYFILE_ALL | COPYFILE_NOFOLLOW);
    }
    return r < 0 && errno == EEXIST ? 0 : r;
}

// Makes each directory above `rest` in the upper, as the lower has it.
static int parents(struct root *it, const char *rest) {
    if (!*rest) return 0;
    char upper[PATH_MAX], lower[PATH_MAX];
    size_t ulen = it->target_len, llen = it->len;
    memcpy(upper, it->target, ulen);
    memcpy(lower, it->path, llen);
    for (const char *at = rest + 1; (at = strchr(at, '/')); at++) {
        size_t part = at - rest;
        if (ulen + part >= PATH_MAX || llen + part >= PATH_MAX) {
            errno = ENAMETOOLONG;
            return -1;
        }
        memcpy(upper + ulen, rest, part);
        upper[ulen + part] = 0;
        memcpy(lower + llen, rest, part);
        lower[llen + part] = 0;
        struct stat st;
        switch (state_of(upper, &st)) {
        case PRESENT:
            if (S_ISDIR(st.st_mode) || S_ISLNK(st.st_mode)) continue;
            errno = ENOTDIR;
            return -1;
        case WHITED: errno = ENOENT; return -1;
        case NOT_DIR: errno = ENOTDIR; return -1;
        case MISSING:
            if (lstat(lower, &st) < 0) return -1;
            if (!S_ISDIR(st.st_mode) && !S_ISLNK(st.st_mode)) {
                errno = ENOTDIR;
                return -1;
            }
            if (copy_up(lower, upper, &st, 0) < 0) return -1;
            written(it);
        }
    }
    return 0;
}

// Whiteouts in the upper directory `upper` for what the lower directory `lower` has and the
// upper does not: what makes a directory made again where one was removed empty.
static void hide_lower(const char *lower, const char *upper) {
    DIR *dir = opendir(lower);
    if (!dir) return;
    struct dirent *entry;
    char path[PATH_MAX];
    struct stat st;
    while ((entry = readdir(dir))) {
        if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
        if (join(path, upper, entry->d_name) < 0) continue;
        if (lstat(path, &st) < 0) whiteout(path);
    }
    closedir(dir);
}

// Whether the directory the program sees at `lower` (with `upper` over it) has nothing in it.
static int empty_dir(const char *lower, const char *upper) {
    char path[PATH_MAX];
    struct stat st;
    struct dirent *entry;
    DIR *dir = opendir(upper);
    if (dir) {
        while ((entry = readdir(dir))) {
            if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
            if (join(path, upper, entry->d_name) == 0 && lstat(path, &st) == 0 &&
                !whiteout_at(path, &st)) {
                closedir(dir);
                return 0;
            }
        }
        closedir(dir);
    }
    dir = opendir(lower);
    if (dir) {
        while ((entry = readdir(dir))) {
            if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
            if (join(path, upper, entry->d_name) < 0 || lstat(path, &st) < 0) {
                closedir(dir);
                return 0;
            }
        }
        closedir(dir);
    }
    return 1;
}

// Removes the upper directory `upper`, which holds whiteouts and empty directories only.
static void remove_upper(const char *upper) {
    DIR *dir = opendir(upper);
    if (dir) {
        struct dirent *entry;
        char path[PATH_MAX];
        while ((entry = readdir(dir))) {
            if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
            if (join(path, upper, entry->d_name) < 0) continue;
            if (entry->d_type == DT_DIR) remove_upper(path);
            else unlink(path);
        }
        closedir(dir);
    }
    rmdir(upper);
}

// The upper directory `upper` made whole: what the lower has below it cloned in, for it to
// be moved as one.
static int complete(const char *lower, const char *upper) {
    DIR *dir = opendir(lower);
    if (!dir) return 0;
    struct dirent *entry;
    char from[PATH_MAX], to[PATH_MAX];
    struct stat st, lst;
    int r = 0;
    while ((entry = readdir(dir)) && r == 0) {
        if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
        if (join(from, lower, entry->d_name) < 0 || join(to, upper, entry->d_name) < 0) continue;
        if (lstat(to, &st) < 0) {
            r = clonefile(from, to, CLONE_NOFOLLOW);
            if (r < 0)
                r = copyfile(from, to, NULL, COPYFILE_ALL | COPYFILE_RECURSIVE | COPYFILE_NOFOLLOW);
        } else if (S_ISDIR(st.st_mode) && lstat(from, &lst) == 0 && S_ISDIR(lst.st_mode)) {
            r = complete(from, to);
        }
    }
    closedir(dir);
    return r;
}

// A directory listing the directory the program sees at `virt` (the lower's entries and
// the upper's over them), for it to be read as one: an entry of the same kind for each.
// What is done through it goes back to `virt` (`reverse`).
static int listing(const char *virt, const char *lower, const char *upper, char *out) {
    unsigned n = atomic_fetch_add(&shared->lists, 1);
    if (snprintf(out, PATH_MAX, "%s/%d.%u%s", lists, getpid(), n, virt) >= PATH_MAX) {
        errno = ENAMETOOLONG;
        return -1;
    }
    // Every directory to it, as `mkdir -p`.
    for (char *at = out + lists_len + 1; (at = strchr(at, '/')); at++) {
        *at = 0;
        mkdir(out, 0700);
        *at = '/';
    }
    if (mkdir(out, 0700) < 0 && errno != EEXIST) return -1;
    char from[PATH_MAX], to[PATH_MAX];
    struct stat st;
    struct dirent *entry;
    for (int pass = 0; pass < 2; pass++) {
        DIR *dir = opendir(pass == 0 ? upper : lower);
        if (!dir) continue;
        while ((entry = readdir(dir))) {
            if (!strcmp(entry->d_name, ".") || !strcmp(entry->d_name, "..")) continue;
            if (join(to, out, entry->d_name) < 0) continue;
            if (pass == 0) {
                if (join(from, upper, entry->d_name) < 0 || lstat(from, &st) < 0 ||
                    whiteout_at(from, &st))
                    continue;
            } else {
                if (join(from, upper, entry->d_name) < 0 || lstat(from, &st) == 0) continue;
                st.st_mode = DTTOIF(entry->d_type);
            }
            if (S_ISDIR(st.st_mode)) {
                mkdir(to, 0700);
            } else if (S_ISLNK(st.st_mode)) {
                symlink(".", to);
            } else {
                int fd = open(to, O_CREAT | O_WRONLY | O_CLOEXEC, 0600);
                if (fd >= 0) close(fd);
            }
        }
        closedir(dir);
    }
    return 0;
}

// What the path in overlay `it` (the rest `rest` of `virt`) is to be named as for `how`.
static enum answer overlay(struct root *it, const char *virt, const char *rest, enum how how,
                           char *real) {
    char upper[PATH_MAX];
    if (snprintf(upper, sizeof upper, "%s%s", it->target, rest) >= PATH_MAX) {
        errno = ENAMETOOLONG;
        return FAIL;
    }
    struct stat st;
    if (how == READ || how == LIST) {
        if (!atomic_load(&shared->written[it->index]) && !atomic_load(&shared->whiteouts))
            return PASS;
        int error;
        switch (state_of(upper, &st)) {
        case WHITED: errno = ENOENT; return FAIL;
        case NOT_DIR: errno = ENOTDIR; return FAIL;
        case PRESENT: {
            struct stat lst;
            if (how == LIST && S_ISDIR(st.st_mode) && stat(virt, &lst) == 0 &&
                S_ISDIR(lst.st_mode) && !hidden(it, rest, &error))
                return listing(virt, virt, upper, real) < 0 ? FAIL : USE;
            strcpy(real, upper);
            return USE;
        }
        case MISSING:
            if (atomic_load(&shared->whiteouts) && hidden(it, rest, &error)) {
                errno = error;
                return FAIL;
            }
            strcpy(real, virt);
            return USE;
        }
    }
    if (parents(it, rest) < 0) return FAIL;
    strcpy(real, upper);
    switch (state_of(upper, &st)) {
    case PRESENT: return USE;
    case NOT_DIR: errno = ENOTDIR; return FAIL;
    case WHITED:
        if (how == CHANGE) {
            errno = ENOENT;
            return FAIL;
        }
        unlink(upper);
        return USE;
    case MISSING:
        if (lstat(virt, &st) < 0) {
            if (how == CHANGE) return FAIL;
            written(it);
            return USE;
        }
        if (copy_up(virt, upper, &st, how == CREATE_EMPTY) < 0) return FAIL;
        written(it);
        return USE;
    }
    return USE;
}

// What a call naming `path` from `dirfd` is to name for `how`: PASS for what it named
// (outside the overlays and the bound directories), USE for `real`, FAIL with `errno` set.
// `virt` is the whole path as the program sees it, or empty.
static enum answer resolve(int dirfd, const char *path, enum how how, char *real, char *virt,
                           struct root **root) {
    *root = NULL;
    virt[0] = 0;
    if (absolute(dirfd, path, virt) < 0) {
        virt[0] = 0;
        return PASS;
    }
    const char *rest;
    struct root *it = where(virt, &rest);
    *root = it;
    if (record_reads && (!it || (it->kind != SCRATCH && it->kind != BIND))) note('R', virt);
    if (!it) return PASS;
    switch (it->kind) {
    case BIND:
        if (snprintf(real, PATH_MAX, "%s%s", it->target, rest) >= PATH_MAX) return PASS;
        return USE;
    case OVERLAY: return overlay(it, virt, rest, how, real);
    default: return PASS;
    }
}

// Notes a write the sandbox refused: `errno` says so, and it was outside the run's own.
static void refused(int r, const char *virt, struct root *root) {
    if (r < 0 && (errno == EPERM || errno == EACCES) && virt[0] &&
        (!root || root->kind == WRITABLE || root->kind == NONE))
        note('W', virt);
}

// ---------------------------------------------------------------------------------------
// Setting up.

static int configured(void) {
    return alive && !busy;
}

static void fork_prepare(void) {
    os_unfair_lock_lock(&seen_lock);
    os_unfair_lock_lock(&cwd_lock);
}

static void fork_after(void) {
    os_unfair_lock_unlock(&cwd_lock);
    os_unfair_lock_unlock(&seen_lock);
}

// The child's thread is another Mach thread than the one that took the locks, and an unfair
// lock aborts the process that unlocks one it does not own: the child starts them afresh.
static void fork_child(void) {
    cwd_lock = OS_UNFAIR_LOCK_INIT;
    seen_lock = OS_UNFAIR_LOCK_INIT;
}

static void image_added(const struct mach_header *header, intptr_t slide) {
    (void)slide;
    Dl_info info;
    if (!dladdr(header, &info) || !info.dli_fname) return;
    // What the shared cache holds is the system's; the rest is read from disk.
    if (_dyld_shared_cache_contains_path(info.dli_fname) || !strcmp(info.dli_fname, self))
        return;
    int was = busy;
    busy = 1;
    char virt[PATH_MAX];
    if (absolute(AT_FDCWD, info.dli_fname, virt) == 0) {
        const char *rest;
        struct root *it = where(virt, &rest);
        if (!it || (it->kind != SCRATCH && it->kind != BIND)) note('R', virt);
    }
    busy = was;
}

static void add_root(const char *path, enum kind kind, int index, const char *target) {
    if (nroots == MAX_ROOTS || !*path) return;
    struct root *it = &roots[nroots++];
    it->path = strdup(path);
    it->len = strlen(path);
    if (it->len > 1 && it->path[it->len - 1] == '/') it->path[--it->len] = 0;
    it->kind = kind;
    it->index = index;
    it->target = target ? strdup(target) : NULL;
    it->target_len = target ? strlen(target) : 0;
}

static int longer(const void *a, const void *b) {
    const struct root *x = a, *y = b;
    return (x->len < y->len) - (x->len > y->len);
}

__attribute__((constructor)) static void start(void) {
    const char *dir = getenv("__RA_JIT_SANDBOX");
    if (!dir || strlen(dir) + 16 >= PATH_MAX) return;
    busy = 1;
    strcpy(scratch, dir);
    scratch_len = strlen(scratch);
    snprintf(lists, sizeof lists, "%s/list", scratch);
    lists_len = strlen(lists);
    char path[PATH_MAX];
    snprintf(path, sizeof path, "%s/state", scratch);
    int fd = open(path, O_RDWR | O_CLOEXEC);
    if (fd < 0) {
        busy = 0;
        return;
    }
    shared = mmap(NULL, sizeof *shared, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    close(fd);
    if (shared == MAP_FAILED) {
        shared = NULL;
        busy = 0;
        return;
    }
    snprintf(path, sizeof path, "%s/config", scratch);
    FILE *config = fopen(path, "r");
    char line[3 * PATH_MAX];
    int record = 0;
    while (config && fgets(line, sizeof line, config)) {
        line[strcspn(line, "\n")] = 0;
        char *field[4] = {line, NULL, NULL, NULL};
        for (int i = 1; i < 4; i++) {
            char *tab = field[i - 1] ? strchr(field[i - 1], '\t') : NULL;
            if (!tab) break;
            *tab = 0;
            field[i] = tab + 1;
        }
        if (!strcmp(field[0], "O") && field[3]) {
            int index = atoi(field[1]);
            if (index >= 0 && index < MAX_OVERLAYS) add_root(field[2], OVERLAY, index, field[3]);
        } else if (!strcmp(field[0], "B") && field[2]) {
            add_root(field[1], BIND, 0, field[2]);
        } else if (!strcmp(field[0], "W") && field[1]) {
            add_root(field[1], WRITABLE, 0, NULL);
        } else if (!strcmp(field[0], "L")) {
            record = 1;
        } else if (!strcmp(field[0], "R")) {
            record_reads = 1;
        }
    }
    if (config) fclose(config);
    add_root(scratch, SCRATCH, 0, NULL);
    qsort(roots, nroots, sizeof roots[0], longer);
    if (record) {
        snprintf(path, sizeof path, "%s/record", scratch);
        fd = open(path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0600);
        if (fd >= 0) {
            // Out of the way of programs that hand out low descriptors themselves.
            record_fd = fcntl(fd, F_DUPFD_CLOEXEC, 512);
            if (record_fd < 0) record_fd = fd;
            else close(fd);
        }
    }
    Dl_info info;
    if (dladdr((const void *)start, &info) && info.dli_fname)
        snprintf(self, sizeof self, "%s", info.dli_fname);
    snprintf(env_insert, sizeof env_insert, "DYLD_INSERT_LIBRARIES=%s", self);
    snprintf(env_dir, sizeof env_dir, "__RA_JIT_SANDBOX=%s", scratch);
    const char *nonce = getenv("__RA_JIT_SANDBOX_EXEC");
    if (nonce) emit("H", nonce, NULL, NULL);
    // The program sees the environment it was given: this library's variables go, and
    // come back in what it runs (`environment`).
    const char *insert = getenv("DYLD_INSERT_LIBRARIES");
    const char *theirs = insert && !strncmp(insert, self, strlen(self)) ? insert + strlen(self) : NULL;
    if (theirs && *theirs == ':' && theirs[1]) setenv("DYLD_INSERT_LIBRARIES", theirs + 1, 1);
    else if (theirs) unsetenv("DYLD_INSERT_LIBRARIES");
    unsetenv("__RA_JIT_SANDBOX");
    unsetenv("__RA_JIT_SANDBOX_EXEC");
    pthread_atfork(fork_prepare, fork_after, fork_child);
    alive = 1;
    if (record_reads) _dyld_register_func_for_add_image(image_added);
    busy = 0;
}

// ---------------------------------------------------------------------------------------
// The calls.

#define BEGIN                                                                              \
    char real[PATH_MAX], virt[PATH_MAX];                                                   \
    struct root *root;                                                                     \
    busy = 1;
#define END(r)                                                                             \
    do {                                                                                   \
        int saved_ = errno;                                                                \
        busy = 0;                                                                          \
        errno = saved_;                                                                    \
        return r;                                                                          \
    } while (0)

static enum how how_open(int flags) {
    if (flags & O_CREAT) return (flags & O_TRUNC) ? CREATE_EMPTY : CREATE;
    if (flags & (O_WRONLY | O_RDWR | O_TRUNC | O_APPEND)) return CHANGE;
    if (flags & O_DIRECTORY) return LIST;
    return READ;
}

static int opened(int dirfd, const char *path, int flags, int mode,
                  int (*call)(int, const char *, int, int)) {
    if (!configured()) return call(dirfd, path, flags, mode);
    BEGIN
    enum how how = how_open(flags);
    enum answer a = resolve(dirfd, path, how, real, virt, &root);
    int r = a == FAIL ? -1 : a == USE ? call(AT_FDCWD, real, flags, mode)
                                      : call(dirfd, path, flags, mode);
    if (how != READ && how != LIST) refused(r, virt, root);
    END(r);
}

static int via_openat(int dirfd, const char *path, int flags, int mode) {
    return openat(dirfd, path, flags, mode);
}
static int via_openat_nocancel(int dirfd, const char *path, int flags, int mode) {
    return openat_nocancel(dirfd, path, flags, mode);
}
static int via_open(int dirfd, const char *path, int flags, int mode) {
    return dirfd == AT_FDCWD ? open(path, flags, mode) : openat(dirfd, path, flags, mode);
}
static int via_open_nocancel(int dirfd, const char *path, int flags, int mode) {
    return dirfd == AT_FDCWD ? open_nocancel(path, flags, mode)
                             : openat_nocancel(dirfd, path, flags, mode);
}
static int via___open(int dirfd, const char *path, int flags, int mode) {
    return dirfd == AT_FDCWD ? __open(path, flags, mode) : __openat(dirfd, path, flags, mode);
}
static int via___open_nocancel(int dirfd, const char *path, int flags, int mode) {
    return dirfd == AT_FDCWD ? __open_nocancel(path, flags, mode)
                             : __openat_nocancel(dirfd, path, flags, mode);
}

static int my_open(const char *path, int flags, ...) {
    va_list ap;
    va_start(ap, flags);
    int mode = (flags & O_CREAT) ? va_arg(ap, int) : 0;
    va_end(ap);
    return opened(AT_FDCWD, path, flags, mode, via_open);
}
static int my_open_nocancel(const char *path, int flags, ...) {
    va_list ap;
    va_start(ap, flags);
    int mode = (flags & O_CREAT) ? va_arg(ap, int) : 0;
    va_end(ap);
    return opened(AT_FDCWD, path, flags, mode, via_open_nocancel);
}
static int my_openat(int dirfd, const char *path, int flags, ...) {
    va_list ap;
    va_start(ap, flags);
    int mode = (flags & O_CREAT) ? va_arg(ap, int) : 0;
    va_end(ap);
    return opened(dirfd, path, flags, mode, via_openat);
}
static int my_openat_nocancel(int dirfd, const char *path, int flags, ...) {
    va_list ap;
    va_start(ap, flags);
    int mode = (flags & O_CREAT) ? va_arg(ap, int) : 0;
    va_end(ap);
    return opened(dirfd, path, flags, mode, via_openat_nocancel);
}
static int my___open(const char *path, int flags, int mode) {
    return opened(AT_FDCWD, path, flags, mode, via___open);
}
static int my___open_nocancel(const char *path, int flags, int mode) {
    return opened(AT_FDCWD, path, flags, mode, via___open_nocancel);
}
static int my___openat(int dirfd, const char *path, int flags, int mode) {
    return opened(dirfd, path, flags, mode, via___open);
}
static int my___openat_nocancel(int dirfd, const char *path, int flags, int mode) {
    return opened(dirfd, path, flags, mode, via___open_nocancel);
}
static int my_creat(const char *path, mode_t mode) {
    return opened(AT_FDCWD, path, O_CREAT | O_TRUNC | O_WRONLY, mode, via_open);
}
INTERPOSE(my_open, open)
INTERPOSE(my_open_nocancel, open_nocancel)
INTERPOSE(my_openat, openat)
INTERPOSE(my_openat_nocancel, openat_nocancel)
INTERPOSE(my___open, __open)
INTERPOSE(my___open_nocancel, __open_nocancel)
INTERPOSE(my___openat, __openat)
INTERPOSE(my___openat_nocancel, __openat_nocancel)
INTERPOSE(my_creat, creat)

// A call that names one path, for `how`, made as `CALL` with `P` the path to name.
#define PATH_CALL(name, how, params, dirfd, path, call_real, call_with, write)               \
    static int my_##name params {                                                          \
        if (!configured()) return call_real;                                               \
        BEGIN                                                                              \
        enum answer a = resolve(dirfd, path, how, real, virt, &root);                       \
        int r = a == FAIL ? -1 : a == USE ? call_with : call_real;                          \
        if (write) refused(r, virt, root);                                                 \
        END(r);                                                                            \
    }                                                                                      \
    INTERPOSE(my_##name, name)

PATH_CALL(stat, READ, (const char *p, struct stat *s), AT_FDCWD, p, stat(p, s), stat(real, s), 0)
PATH_CALL(lstat, READ, (const char *p, struct stat *s), AT_FDCWD, p, lstat(p, s), lstat(real, s), 0)
PATH_CALL(fstatat, READ, (int d, const char *p, struct stat *s, int f), d, p,
          fstatat(d, p, s, f), fstatat(AT_FDCWD, real, s, f), 0)
PATH_CALL(faccessat, READ, (int d, const char *p, int m, int f), d, p, faccessat(d, p, m, f),
          faccessat(AT_FDCWD, real, m, f), 0)
PATH_CALL(readlink, READ, (const char *p, char *b, size_t n), AT_FDCWD, p, readlink(p, b, n),
          readlink(real, b, n), 0)
PATH_CALL(readlinkat, READ, (int d, const char *p, char *b, size_t n), d, p,
          readlinkat(d, p, b, n), readlinkat(AT_FDCWD, real, b, n), 0)
PATH_CALL(getattrlist, READ, (const char *p, void *l, void *b, size_t n, unsigned int o),
          AT_FDCWD, p, getattrlist(p, l, b, n, o), getattrlist(real, l, b, n, o), 0)
PATH_CALL(getattrlistat, READ,
          (int d, const char *p, void *l, void *b, size_t n, unsigned long o), d, p,
          getattrlistat(d, p, l, b, n, o), getattrlistat(AT_FDCWD, real, l, b, n, o), 0)
PATH_CALL(statfs, READ, (const char *p, struct statfs *b), AT_FDCWD, p, statfs(p, b),
          statfs(real, b), 0)
PATH_CALL(getxattr, READ, (const char *p, const char *n, void *v, size_t s, u_int32_t o, int f),
          AT_FDCWD, p, getxattr(p, n, v, s, o, f), getxattr(real, n, v, s, o, f), 0)
PATH_CALL(listxattr, READ, (const char *p, char *n, size_t s, int f), AT_FDCWD, p,
          listxattr(p, n, s, f), listxattr(real, n, s, f), 0)
PATH_CALL(pathconf, READ, (const char *p, int n), AT_FDCWD, p, pathconf(p, n), pathconf(real, n),
          0)
PATH_CALL(chmod, CHANGE, (const char *p, mode_t m), AT_FDCWD, p, chmod(p, m), chmod(real, m), 1)
PATH_CALL(fchmodat, CHANGE, (int d, const char *p, mode_t m, int f), d, p, fchmodat(d, p, m, f),
          fchmodat(AT_FDCWD, real, m, f), 1)
PATH_CALL(chown, CHANGE, (const char *p, uid_t u, gid_t g), AT_FDCWD, p, chown(p, u, g),
          chown(real, u, g), 1)
PATH_CALL(lchown, CHANGE, (const char *p, uid_t u, gid_t g), AT_FDCWD, p, lchown(p, u, g),
          lchown(real, u, g), 1)
PATH_CALL(chflags, CHANGE, (const char *p, unsigned int f), AT_FDCWD, p, chflags(p, f),
          chflags(real, f), 1)
PATH_CALL(utimes, CHANGE, (const char *p, const struct timeval t[2]), AT_FDCWD, p, utimes(p, t),
          utimes(real, t), 1)
PATH_CALL(lutimes, CHANGE, (const char *p, const struct timeval t[2]), AT_FDCWD, p, lutimes(p, t),
          lutimes(real, t), 1)
PATH_CALL(utimensat, CHANGE, (int d, const char *p, const struct timespec t[2], int f), d, p,
          utimensat(d, p, t, f), utimensat(AT_FDCWD, real, t, f), 1)
PATH_CALL(truncate, CHANGE, (const char *p, off_t n), AT_FDCWD, p, truncate(p, n),
          truncate(real, n), 1)
PATH_CALL(setattrlist, CHANGE, (const char *p, void *l, void *b, size_t n, unsigned int o),
          AT_FDCWD, p, setattrlist(p, l, b, n, o), setattrlist(real, l, b, n, o), 1)
PATH_CALL(setxattr,
          CHANGE, (const char *p, const char *n, const void *v, size_t s, u_int32_t o, int f),
          AT_FDCWD, p, setxattr(p, n, v, s, o, f), setxattr(real, n, v, s, o, f), 1)
PATH_CALL(removexattr, CHANGE, (const char *p, const char *n, int f), AT_FDCWD, p,
          removexattr(p, n, f), removexattr(real, n, f), 1)
PATH_CALL(symlink, CREATE, (const char *t, const char *p), AT_FDCWD, p, symlink(t, p),
          symlink(t, real), 1)
PATH_CALL(symlinkat, CREATE, (const char *t, int d, const char *p), d, p, symlinkat(t, d, p),
          symlinkat(t, AT_FDCWD, real), 1)
PATH_CALL(mkfifo, CREATE, (const char *p, mode_t m), AT_FDCWD, p, mkfifo(p, m), mkfifo(real, m),
          1)
PATH_CALL(fclonefileat, CREATE, (int s, int d, const char *p, uint32_t f), d, p,
          fclonefileat(s, d, p, f), fclonefileat(s, AT_FDCWD, real, f), 1)

// `access`, which is also the harness's marker: `/.ra-jit-test/<name>` names the test the
// thread runs from now on, `/.ra-jit-test/` none.
static int my_access(const char *p, int m) {
    if (alive && p && strncmp(p, MARK, sizeof MARK - 1) == 0) {
        const char *name = p + sizeof MARK - 1;
        size_t len = strlen(name);
        if (len >= sizeof test) len = sizeof test - 1;
        memcpy(test, name, len);
        test[len] = 0;
        errno = ENOENT;
        return -1;
    }
    if (!configured()) return access(p, m);
    BEGIN
    enum answer a = resolve(AT_FDCWD, p, READ, real, virt, &root);
    int r = a == FAIL ? -1 : a == USE ? access(real, m) : access(p, m);
    END(r);
}
INTERPOSE(my_access, access)

// Makes a directory; one made again where the lower had one that was removed is empty.
static int made(int d, const char *p, mode_t m) {
    if (!configured()) return mkdirat(d, p, m);
    BEGIN
    enum answer a = resolve(d, p, CREATE, real, virt, &root);
    struct stat st;
    int again = a == USE && root && root->kind == OVERLAY && lstat(virt, &st) == 0;
    int r = a == FAIL ? -1 : a == USE ? mkdir(real, m) : mkdirat(d, p, m);
    if (r == 0 && again && S_ISDIR(st.st_mode)) hide_lower(virt, real);
    refused(r, virt, root);
    END(r);
}
static int my_mkdir(const char *p, mode_t m) {
    return made(AT_FDCWD, p, m);
}
static int my_mkdirat(int d, const char *p, mode_t m) {
    return made(d, p, m);
}
INTERPOSE(my_mkdir, mkdir)
INTERPOSE(my_mkdirat, mkdirat)

// Removes a file, or a directory with `dir`.
static int removed(int d, const char *p, int dir) {
    int flag = dir ? AT_REMOVEDIR : 0;
    if (!configured()) return unlinkat(d, p, flag);
    BEGIN
    resolve(d, p, READ, real, virt, &root);
    int r;
    if (!root || root->kind != OVERLAY) {
        if (root && root->kind == BIND)
            r = dir ? rmdir(real) : unlink(real);
        else
            r = unlinkat(d, p, flag);
        refused(r, virt, root);
        END(r);
    }
    const char *rest = virt + root->len;
    char upper[PATH_MAX];
    snprintf(upper, sizeof upper, "%s%s", root->target, rest);
    struct stat ust, lst;
    int error;
    if (parents(root, rest) < 0) END(-1);
    enum state state = state_of(upper, &ust);
    int lower = lstat(virt, &lst) == 0;
    if (state == WHITED || state == NOT_DIR ||
        (state == MISSING && (!lower || (atomic_load(&shared->whiteouts) &&
                                         hidden(root, rest, &error))))) {
        errno = state == NOT_DIR ? ENOTDIR : ENOENT;
        END(-1);
    }
    const struct stat *seen_st = state == PRESENT ? &ust : &lst;
    if (dir) {
        if (!S_ISDIR(seen_st->st_mode)) {
            errno = ENOTDIR;
            END(-1);
        }
        if (!empty_dir(lower && S_ISDIR(lst.st_mode) ? virt : "/var/empty", upper)) {
            errno = ENOTEMPTY;
            END(-1);
        }
        if (state == PRESENT) remove_upper(upper);
    } else {
        if (S_ISDIR(seen_st->st_mode)) {
            errno = EPERM;
            END(-1);
        }
        if (state == PRESENT && unlink(upper) < 0) END(-1);
    }
    r = 0;
    if (lower) {
        r = whiteout(upper);
        written(root);
    }
    END(r);
}
static int my_unlink(const char *p) {
    return removed(AT_FDCWD, p, 0);
}
static int my_rmdir(const char *p) {
    return removed(AT_FDCWD, p, 1);
}
static int my_unlinkat(int d, const char *p, int f) {
    return removed(d, p, f & AT_REMOVEDIR);
}
INTERPOSE(my_unlink, unlink)
INTERPOSE(my_rmdir, rmdir)
INTERPOSE(my_unlinkat, unlinkat)

// Moves `from` to `to`. A directory of an overlay is made whole in the upper first, moved
// there, and a whiteout left where it was.
static int moved(int fd, const char *from, int td, const char *to, unsigned flags) {
    if (!configured()) return renameatx_np(fd, from, td, to, flags);
    busy = 1;
    char real_from[PATH_MAX], real_to[PATH_MAX], virt_from[PATH_MAX], virt_to[PATH_MAX];
    struct root *root_from, *root_to;
    enum answer a = resolve(fd, from, READ, real_from, virt_from, &root_from);
    if (a == FAIL) END(-1);
    int from_overlay = root_from && root_from->kind == OVERLAY;
    struct stat st;
    int lower_from = from_overlay && lstat(virt_from, &st) == 0;
    if (from_overlay) {
        // The source as the program sees it: from the upper or the lower.
        if (lstat(a == USE ? real_from : virt_from, &st) < 0) END(-1);
        a = resolve(fd, from, CHANGE, real_from, virt_from, &root_from);
        if (a == FAIL) END(-1);
        if (S_ISDIR(st.st_mode) && lower_from && complete(virt_from, real_from) < 0) END(-1);
    }
    enum answer b = resolve(td, to, CREATE, real_to, virt_to, &root_to);
    if (b == FAIL) END(-1);
    int to_overlay = root_to && root_to->kind == OVERLAY;
    struct stat tst;
    int lower_dir_to = to_overlay && lstat(virt_to, &tst) == 0 && S_ISDIR(tst.st_mode);
    // The upper copy of what is replaced is a clone, which the move replaces as it would.
    int r = renameatx_np(a == USE ? AT_FDCWD : fd, a == USE ? real_from : from,
                         b == USE ? AT_FDCWD : td, b == USE ? real_to : to, flags);
    if (r == 0 && lower_from && !(flags & RENAME_SWAP)) {
        whiteout(real_from);
        written(root_from);
    }
    if (r == 0 && lower_dir_to) hide_lower(virt_to, real_to);
    refused(r, virt_from, root_from);
    refused(r, virt_to, root_to);
    END(r);
}
static int my_rename(const char *from, const char *to) {
    return moved(AT_FDCWD, from, AT_FDCWD, to, 0);
}
static int my_renameat(int fd, const char *from, int td, const char *to) {
    return moved(fd, from, td, to, 0);
}
static int my_renamex_np(const char *from, const char *to, unsigned flags) {
    return moved(AT_FDCWD, from, AT_FDCWD, to, flags);
}
static int my_renameatx_np(int fd, const char *from, int td, const char *to, unsigned flags) {
    return moved(fd, from, td, to, flags);
}
INTERPOSE(my_rename, rename)
INTERPOSE(my_renameat, renameat)
INTERPOSE(my_renamex_np, renamex_np)
INTERPOSE(my_renameatx_np, renameatx_np)

// A link or a clone: `from` is read (and for a link brought up into the upper, where the
// link is made), `to` made.
static int linked(int fd, const char *from, int td, const char *to, int flags, int clone) {
    if (!configured())
        return clone ? clonefileat(fd, from, td, to, flags) : linkat(fd, from, td, to, flags);
    BEGIN
    char real_to[PATH_MAX], virt_to[PATH_MAX];
    struct root *root_to;
    enum answer a = resolve(fd, from, clone ? READ : CHANGE, real, virt, &root);
    if (a == FAIL) END(-1);
    enum answer b = resolve(td, to, CREATE, real_to, virt_to, &root_to);
    if (b == FAIL) END(-1);
    int f1 = a == USE ? AT_FDCWD : fd, f2 = b == USE ? AT_FDCWD : td;
    const char *p1 = a == USE ? real : from, *p2 = b == USE ? real_to : to;
    int r = clone ? clonefileat(f1, p1, f2, p2, flags) : linkat(f1, p1, f2, p2, flags);
    refused(r, virt_to, root_to);
    END(r);
}
static int my_link(const char *from, const char *to) {
    return linked(AT_FDCWD, from, AT_FDCWD, to, 0, 0);
}
static int my_linkat(int fd, const char *from, int td, const char *to, int flags) {
    return linked(fd, from, td, to, flags, 0);
}
static int my_clonefile(const char *from, const char *to, uint32_t flags) {
    return linked(AT_FDCWD, from, AT_FDCWD, to, flags, 1);
}
static int my_clonefileat(int fd, const char *from, int td, const char *to, uint32_t flags) {
    return linked(fd, from, td, to, flags, 1);
}
INTERPOSE(my_link, link)
INTERPOSE(my_linkat, linkat)
INTERPOSE(my_clonefile, clonefile)
INTERPOSE(my_clonefileat, clonefileat)

// The working directory, which the program knows by the path it named.
static int my_chdir(const char *p) {
    if (!configured()) return chdir(p);
    BEGIN
    enum answer a = resolve(AT_FDCWD, p, READ, real, virt, &root);
    int r = a == FAIL ? -1 : a == USE ? chdir(real) : chdir(p);
    os_unfair_lock_lock(&cwd_lock);
    cwd_known = 0;
    os_unfair_lock_unlock(&cwd_lock);
    END(r);
}
static int my_fchdir(int fd) {
    int r = fchdir(fd);
    if (alive) {
        os_unfair_lock_lock(&cwd_lock);
        cwd_known = 0;
        os_unfair_lock_unlock(&cwd_lock);
    }
    return r;
}
static char *my_getcwd(char *buf, size_t size) {
    if (!configured()) return getcwd(buf, size);
    busy = 1;
    char path[PATH_MAX];
    char *r = getcwd(path, sizeof path);
    if (r) {
        reverse(path);
        size_t len = strlen(path) + 1;
        if (!buf) {
            buf = malloc(size > len ? size : len);
            if (!buf) END(NULL);
        } else if (size < len) {
            errno = ERANGE;
            END(NULL);
        }
        memcpy(buf, path, len);
        r = buf;
    }
    END(r);
}
INTERPOSE(my_chdir, chdir)
INTERPOSE(my_fchdir, fchdir)
INTERPOSE(my_getcwd, getcwd)

// `F_GETPATH` says the path the program knows.
static int got_path(int fd, int cmd, void *arg, int r) {
    if (r != -1 && cmd == F_GETPATH && alive && arg) reverse(arg);
    return r;
}
static int my_fcntl(int fd, int cmd, ...) {
    va_list ap;
    va_start(ap, cmd);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    return got_path(fd, cmd, arg, fcntl(fd, cmd, arg));
}
static int my_fcntl_nocancel(int fd, int cmd, ...) {
    va_list ap;
    va_start(ap, cmd);
    void *arg = va_arg(ap, void *);
    va_end(ap);
    return got_path(fd, cmd, arg, fcntl_nocancel(fd, cmd, arg));
}
INTERPOSE(my_fcntl, fcntl)
INTERPOSE(my_fcntl_nocancel, fcntl_nocancel)

// ---------------------------------------------------------------------------------------
// Programs.

static _Atomic uint32_t execs;

// How many entries `environment` adds to those it keeps: `DYLD_INSERT_LIBRARIES`, the
// directory and the nonce.
#define ADDED 3

// `envp` with this library's variables in it, which a program that clears its child's
// environment would otherwise take away, and the nonce `nonce` for the child to say it loaded.
// `out` has `room` entries, which is `count(envp) + ADDED + 1`: every entry of `envp` kept, the
// ones added and the terminator.
static void environment(char *const envp[], char **out, size_t room, char *nonce_var,
                        char *insert_var, const char *nonce) {
    size_t n = 0;
    const char *theirs = NULL;
    // Never more than `count(envp)` are kept, so the guard holds for the room it is given; it
    // keeps the additions and the terminator in bounds should it not.
    for (char *const *it = envp; it && *it && n + ADDED + 1 < room; it++) {
        if (!strncmp(*it, "__RA_JIT_SANDBOX=", 17) || !strncmp(*it, "__RA_JIT_SANDBOX_EXEC=", 22))
            continue;
        if (!strncmp(*it, "DYLD_INSERT_LIBRARIES=", 22)) {
            theirs = *it + 22;
            continue;
        }
        out[n++] = *it;
    }
    const char *ours = env_insert + 22;
    if (theirs && *theirs && !strstr(theirs, ours))
        snprintf(insert_var, PATH_MAX * 2, "DYLD_INSERT_LIBRARIES=%s:%s", ours, theirs);
    else
        snprintf(insert_var, PATH_MAX * 2, "%s", env_insert);
    snprintf(nonce_var, 64, "__RA_JIT_SANDBOX_EXEC=%s", nonce);
    out[n++] = insert_var;
    out[n++] = env_dir;
    out[n++] = nonce_var;
    out[n] = NULL; // n + 1 <= room: the loop left ADDED + 1 entries
}

static size_t count(char *const envp[]) {
    size_t n = 0;
    while (envp && envp[n]) n++;
    return n;
}

// A name for one program run, told apart from those of earlier images of the same process.
static void new_nonce(char *out) {
    static uint32_t image;
    if (!image) image = arc4random() | 1;
    snprintf(out, 32, "%d.%x.%u", getpid(), image, atomic_fetch_add(&execs, 1));
}

static int my_execve(const char *path, char *const argv[], char *const envp[]) {
    if (!configured()) return execve(path, argv, envp);
    BEGIN
    enum answer a = resolve(AT_FDCWD, path, READ, real, virt, &root);
    if (a == FAIL) END(-1);
    size_t room = count(envp) + ADDED + 1;
    char *env[room];
    char nonce[32], nonce_var[64], insert_var[PATH_MAX * 2];
    new_nonce(nonce);
    environment(envp, env, room, nonce_var, insert_var, nonce);
    emit("E", nonce, test, virt[0] ? virt : path);
    int r = execve(a == USE ? real : path, argv, env);
    emit("F", nonce, NULL, NULL);
    END(r);
}
INTERPOSE(my_execve, execve)

static int spawned(pid_t *pid, const char *path, const posix_spawn_file_actions_t *actions,
                   const posix_spawnattr_t *attr, char *const argv[], char *const envp[]) {
    char real[PATH_MAX], virt[PATH_MAX];
    struct root *root;
    enum answer a = resolve(AT_FDCWD, path, READ, real, virt, &root);
    if (a == FAIL) return errno;
    size_t room = count(envp) + ADDED + 1;
    char *env[room];
    char nonce[32], nonce_var[64], insert_var[PATH_MAX * 2];
    new_nonce(nonce);
    environment(envp, env, room, nonce_var, insert_var, nonce);
    pid_t child;
    int r = posix_spawn(&child, a == USE ? real : path, actions, attr, argv, env);
    if (r == 0) {
        emit("E", nonce, test, virt[0] ? virt : path);
        if (pid) *pid = child;
    }
    return r;
}

static int my_posix_spawn(pid_t *pid, const char *path, const posix_spawn_file_actions_t *actions,
                          const posix_spawnattr_t *attr, char *const argv[], char *const envp[]) {
    if (!configured()) return posix_spawn(pid, path, actions, attr, argv, envp);
    busy = 1;
    int r = spawned(pid, path, actions, attr, argv, envp);
    busy = 0;
    return r;
}

// `posix_spawnp` looks for the program along `PATH` itself, each candidate a path the
// program named.
static int my_posix_spawnp(pid_t *pid, const char *file, const posix_spawn_file_actions_t *actions,
                           const posix_spawnattr_t *attr, char *const argv[], char *const envp[]) {
    if (!configured() || !file || strchr(file, '/'))
        return configured() ? my_posix_spawn(pid, file, actions, attr, argv, envp)
                            : posix_spawnp(pid, file, actions, attr, argv, envp);
    busy = 1;
    const char *search = getenv("PATH");
    if (!search) search = "/usr/bin:/bin:/usr/sbin:/sbin";
    int r = ENOENT;
    char candidate[PATH_MAX], real[PATH_MAX], virt[PATH_MAX];
    struct root *root;
    for (const char *at = search;; at++) {
        const char *end = strchr(at, ':');
        size_t len = end ? (size_t)(end - at) : strlen(at);
        if (len == 0) {
            candidate[0] = '.';
            len = 1;
        } else if (len < PATH_MAX) {
            memcpy(candidate, at, len);
        } else {
            // A directory too long for a path: no program is found there.
            if (!end) break;
            at = end;
            continue;
        }
        if (len + strlen(file) + 2 < PATH_MAX) {
            candidate[len] = '/';
            strcpy(candidate + len + 1, file);
            enum answer a = resolve(AT_FDCWD, candidate, READ, real, virt, &root);
            struct stat st;
            if (a != FAIL && stat(a == USE ? real : candidate, &st) == 0 && S_ISREG(st.st_mode) &&
                access(a == USE ? real : candidate, X_OK) == 0) {
                r = spawned(pid, candidate, actions, attr, argv, envp);
                if (r != ENOEXEC) break;
            }
        }
        if (!end) break;
        at = end;
    }
    busy = 0;
    return r;
}
INTERPOSE(my_posix_spawn, posix_spawn)
INTERPOSE(my_posix_spawnp, posix_spawnp)

// ---------------------------------------------------------------------------------------
// The network.

// How long what `outside` writes may be: a Unix socket's path and its prefix.
#define WHERE (sizeof(((struct sockaddr_un *)0)->sun_path) + 8)

// The address `addr` names when it is not this machine's loopback, written to `out` (`WHERE`
// bytes): `unix:` and the path for a Unix socket's, which Seatbelt allows only below the
// run's own and writable directories.
static int outside(const struct sockaddr *addr, socklen_t len, char *out) {
    if (!addr) return 0;
    if (addr->sa_family == AF_UNIX && len > offsetof(struct sockaddr_un, sun_path)) {
        const struct sockaddr_un *un = (const void *)addr;
        size_t most = len - offsetof(struct sockaddr_un, sun_path);
        if (most > sizeof un->sun_path) most = sizeof un->sun_path;
        size_t n = strnlen(un->sun_path, most);
        if (n == 0) return 0;
        snprintf(out, WHERE, "unix:%.*s", (int)n, un->sun_path);
        return 1;
    }
    if (addr->sa_family == AF_INET && len >= sizeof(struct sockaddr_in)) {
        const struct sockaddr_in *in = (const void *)addr;
        uint32_t ip = ntohl(in->sin_addr.s_addr);
        if ((ip >> 24) == 127 || ip == 0) return 0;
        snprintf(out, 64, "%u.%u.%u.%u:%u", ip >> 24, (ip >> 16) & 255, (ip >> 8) & 255, ip & 255,
                 ntohs(in->sin_port));
        return 1;
    }
    if (addr->sa_family == AF_INET6 && len >= sizeof(struct sockaddr_in6)) {
        const struct sockaddr_in6 *in = (const void *)addr;
        const uint8_t *b = in->sin6_addr.s6_addr;
        if (IN6_IS_ADDR_LOOPBACK(&in->sin6_addr) || IN6_IS_ADDR_UNSPECIFIED(&in->sin6_addr))
            return 0;
        if (IN6_IS_ADDR_V4MAPPED(&in->sin6_addr) && b[12] == 127) return 0;
        int at = snprintf(out, 64, "[");
        for (int i = 0; i < 16; i += 2)
            at += snprintf(out + at, 64 - at, "%s%x", i ? ":" : "", (b[i] << 8) | b[i + 1]);
        snprintf(out + at, 64 - at, "]:%u", ntohs(in->sin6_port));
        return 1;
    }
    return 0;
}

static void reached(int r, const struct sockaddr *addr, socklen_t len) {
    char where_[WHERE];
    if (r < 0 && (errno == EPERM || errno == EACCES) && alive && outside(addr, len, where_)) {
        int saved = errno;
        note('N', where_);
        errno = saved;
    }
}

static int my_connect(int s, const struct sockaddr *a, socklen_t n) {
    int r = connect(s, a, n);
    reached(r, a, n);
    return r;
}
static int my_connect_nocancel(int s, const struct sockaddr *a, socklen_t n) {
    int r = connect_nocancel(s, a, n);
    reached(r, a, n);
    return r;
}
static ssize_t my_sendto(int s, const void *b, size_t l, int f, const struct sockaddr *a,
                         socklen_t n) {
    ssize_t r = sendto(s, b, l, f, a, n);
    reached((int)(r < 0 ? -1 : 0), a, n);
    return r;
}
static ssize_t my_sendto_nocancel(int s, const void *b, size_t l, int f, const struct sockaddr *a,
                                  socklen_t n) {
    ssize_t r = sendto_nocancel(s, b, l, f, a, n);
    reached((int)(r < 0 ? -1 : 0), a, n);
    return r;
}
static ssize_t my_sendmsg(int s, const struct msghdr *m, int f) {
    ssize_t r = sendmsg(s, m, f);
    if (m) reached((int)(r < 0 ? -1 : 0), m->msg_name, m->msg_namelen);
    return r;
}
static ssize_t my_sendmsg_nocancel(int s, const struct msghdr *m, int f) {
    ssize_t r = sendmsg_nocancel(s, m, f);
    if (m) reached((int)(r < 0 ? -1 : 0), m->msg_name, m->msg_namelen);
    return r;
}
INTERPOSE(my_connect, connect)
INTERPOSE(my_connect_nocancel, connect_nocancel)
INTERPOSE(my_sendto, sendto)
INTERPOSE(my_sendto_nocancel, sendto_nocancel)
INTERPOSE(my_sendmsg, sendmsg)
INTERPOSE(my_sendmsg_nocancel, sendmsg_nocancel)
