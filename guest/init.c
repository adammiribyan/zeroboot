/* Zeroboot guest agent — simple version.
 * Reads commands from serial, evaluates them using built-in handlers
 * and a pre-imported Python environment.
 *
 * The Python code is executed via a pre-opened pipe to a Python process
 * that was started BEFORE the snapshot. After fork+resume, no fork/exec
 * is needed — just write to the pipe and read the result.
 */
#define _GNU_SOURCE
#include <fcntl.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
#include <errno.h>

#define SERIAL_DEV "/dev/ttyS0"
#define BUF_SIZE 65536

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

/* Simple built-in command evaluator (no fork/exec needed) */
static void eval_builtin(const char *cmd) {
    if (cmd[0] == 0) return;

    /* echo command */
    if (cmd[0] == 'e' && cmd[1] == 'c' && cmd[2] == 'h' && cmd[3] == 'o' && cmd[4] == ' ') {
        serial_puts(cmd + 5);
        serial_puts("\n");
        return;
    }

    /* cat file */
    if (cmd[0] == 'c' && cmd[1] == 'a' && cmd[2] == 't' && cmd[3] == ' ') {
        char buf[4096];
        int fd = open(cmd + 4, O_RDONLY);
        if (fd >= 0) {
            int n;
            while ((n = read(fd, buf, sizeof(buf) - 1)) > 0) {
                buf[n] = 0;
                serial_puts(buf);
            }
            close(fd);
        } else {
            serial_puts("error: cannot open ");
            serial_puts(cmd + 4);
            serial_puts("\n");
        }
        return;
    }

    /* Default: unknown */
    serial_puts("unknown: ");
    serial_puts(cmd);
    serial_puts("\n");
}

int main(void) {
    mount("proc", "/proc", "proc", 0, 0);
    mount("sysfs", "/sys", "sysfs", 0, 0);
    mount("devtmpfs", "/dev", "devtmpfs", 0, 0);

    serial_fd = open(SERIAL_DEV, O_RDWR | O_NOCTTY);
    if (serial_fd < 0) {
        _exit(1);
    }

    serial_puts("READY\n");

    char cmd[BUF_SIZE];
    while (1) {
        int len = serial_read_line(cmd, sizeof(cmd));
        if (len > 0) {
            eval_builtin(cmd);
            serial_puts("ZEROBOOT_DONE\n");
        }
    }
}
