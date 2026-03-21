/* Zeroboot guest agent — minimal C init (PID 1).
 * Mounts filesystems, then loops reading commands from serial.
 * Handles __ENTROPY__ commands to reseed the kernel CRNG.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <unistd.h>

#define SERIAL_DEV "/dev/ttyS0"
#define BUF_SIZE 65536

#define RNDADDENTROPY  0x40085203
#define RNDRESEEDCRNG  0x5207

#define ENTROPY_PREFIX     "__ENTROPY__"
#define ENTROPY_PREFIX_LEN 11

static int serial_fd;

static void serial_puts(const char *s) {
    int len = 0;
    while (s[len]) len++;
    write(serial_fd, s, len);
}

static int serial_read_line(char *buf, int size) {
    int i = 0;
    while (i < size - 1) {
        char c;
        int n = read(serial_fd, &c, 1);
        if (n <= 0) continue;
        if (c == '\n' || c == '\r') {
            if (i == 0) continue;
            buf[i] = 0;
            return i;
        }
        buf[i++] = c;
    }
    buf[i] = 0;
    return i;
}

static int hex_to_bytes(const char *hex, unsigned char *out, int max_out) {
    int i = 0;
    while (hex[0] && hex[1] && i < max_out) {
        unsigned char hi = hex[0], lo = hex[1];
        hi = (hi >= 'a') ? (hi - 'a' + 10) : (hi >= 'A') ? (hi - 'A' + 10) : (hi - '0');
        lo = (lo >= 'a') ? (lo - 'a' + 10) : (lo >= 'A') ? (lo - 'A' + 10) : (lo - '0');
        out[i++] = (hi << 4) | lo;
        hex += 2;
    }
    return i;
}

static int reseed_kernel(const unsigned char *seed, int seed_len) {
    if (seed_len < 16) return -1;
    int fd = open("/dev/urandom", O_RDWR);
    if (fd < 0) return -1;
    struct {
        int entropy_count;
        int buf_size;
        unsigned char buf[32];
    } info;
    info.entropy_count = seed_len * 8;
    info.buf_size = seed_len;
    memcpy(info.buf, seed, seed_len > 32 ? 32 : seed_len);
    int ret = 0;
    if (ioctl(fd, RNDADDENTROPY, &info) < 0) ret = -1;
    ioctl(fd, RNDRESEEDCRNG, 0);
    close(fd);
    return ret;
}

static void eval_builtin(const char *cmd) {
    if (cmd[0] == 0) return;
    if (cmd[0] == 'e' && cmd[1] == 'c' && cmd[2] == 'h' && cmd[3] == 'o' && cmd[4] == ' ') {
        serial_puts(cmd + 5);
        serial_puts("\n");
        return;
    }
    if (cmd[0] == 'c' && cmd[1] == 'a' && cmd[2] == 't' && cmd[3] == ' ') {
        char buf[4096];
        int fd = open(cmd + 4, O_RDONLY);
        if (fd >= 0) {
            int n;
            while ((n = read(fd, buf, sizeof(buf) - 1)) > 0) { buf[n] = 0; serial_puts(buf); }
            close(fd);
        } else {
            serial_puts("error: cannot open ");
            serial_puts(cmd + 4);
            serial_puts("\n");
        }
        return;
    }
    serial_puts("unknown: ");
    serial_puts(cmd);
    serial_puts("\n");
}

int main(void) {
    mount("proc", "/proc", "proc", 0, 0);
    mount("sysfs", "/sys", "sysfs", 0, 0);
    mount("devtmpfs", "/dev", "devtmpfs", 0, 0);

    serial_fd = open(SERIAL_DEV, O_RDWR | O_NOCTTY);
    if (serial_fd < 0) _exit(1);

    serial_puts("READY\n");

    char cmd[BUF_SIZE];
    while (1) {
        int len = serial_read_line(cmd, sizeof(cmd));
        if (len > 0) {
            /* Entropy reseed: decode hex, reseed kernel, no ZEROBOOT_DONE */
            if (len > ENTROPY_PREFIX_LEN &&
                memcmp(cmd, ENTROPY_PREFIX, ENTROPY_PREFIX_LEN) == 0) {
                unsigned char seed[32];
                int seed_len = hex_to_bytes(
                    cmd + ENTROPY_PREFIX_LEN, seed, sizeof(seed));
                reseed_kernel(seed, seed_len);
                continue;
            }
            eval_builtin(cmd);
            serial_puts("ZEROBOOT_DONE\n");
        }
    }
}
