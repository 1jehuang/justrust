#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <linux/random.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int failed;
static int command(const char *name, char *const argv[], int expect_success) {
    fflush(NULL);
    pid_t p = fork();
    if (p == 0) {
        alarm(1800);
        char *env[] = {"PATH=/", "HOME=/empty", "LANG=C", "LC_ALL=C", "TZ=UTC", NULL};
        execve(argv[0], argv, env);
        _exit(127);
    }
    int status = 0;
    if (p < 0 || waitpid(p, &status, 0) != p) status = -1;
    if (status >= 0 && WIFSIGNALED(status))
        printf("CAPSULE SIGNAL %s %d\n", name, WTERMSIG(status));
    int code = status >= 0 && WIFEXITED(status) ? WEXITSTATUS(status) : 255;
    printf("CAPSULE RESULT %s %d\n", name, code);
    if ((code == 0) != expect_success || code == 127 || code == 255) failed = 1;
    return code;
}
int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    if (mount("proc", "/proc", "proc", 0, NULL) ||
        mount("devtmpfs", "/dev", "devtmpfs", 0, NULL)) {
        perror("mount"); failed = 1; goto done;
    }
    /* Owned, public experiment entropy only. This is NOT secret entropy. */
    struct { int entropy_count; int buf_size; unsigned char bytes[64]; } seed;
    seed.entropy_count = 512; seed.buf_size = 64;
    for (int i = 0; i < 64; i++) seed.bytes[i] = (unsigned char)(i * 17 + 3);
    int random = open("/dev/random", O_RDWR);
    if (random < 0 || ioctl(random, RNDADDENTROPY, &seed)) {
        perror("fixed entropy"); failed = 1; goto done;
    }
    close(random);
    puts("CAPSULE ENTROPY public-fixed-64-bytes-rndaddentropy-512");
    char *check[] = {"/compiler", "/work/valid.rs", "--workers", "1", "--check", NULL};
    char *build[] = {"/compiler", "/work/valid.rs", "--workers", "1", "-o", "/owned.elf", NULL};
    char *run[] = {"/owned.elf", NULL};
    char *badcheck[] = {"/compiler", "/work/invalid.rs", "--workers", "1", "--check", NULL};
    char *badbuild[] = {"/compiler", "/work/invalid.rs", "--workers", "1", "-o", "/invalid.elf", NULL};
    char *missing[] = {"/compiler", "/work/missing.rs", "--check", NULL};
    command("check", check, 1);
    command("build", build, 1);
    if (access("/owned.elf", X_OK)) failed = 1;
    command("run", run, 1);
    command("invalid-check", badcheck, 0);
    command("invalid-build", badbuild, 0);
    command("missing-source", missing, 0);
    command("existing-output", build, 0);
    if (!access("/invalid.elf", F_OK)) failed = 1;
    FILE *f = fopen("/owned.elf", "rb");
    if (!f) { failed = 1; goto done; }
    struct stat st;
    if (fstat(fileno(f), &st) || st.st_size > 1048576 || st.st_size < 4) {
        fclose(f); failed = 1; goto done;
    }
    printf("CAPSULE ELF ");
    int c;
    while ((c = fgetc(f)) != EOF) printf("%02x", c);
    puts(""); fclose(f);
done:
    printf("CAPSULE DONE %s\n", failed ? "FAIL" : "PASS");
    sync(); reboot(RB_POWER_OFF);
    return 1;
}
