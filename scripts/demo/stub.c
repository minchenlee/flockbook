// Demo agent stub for scripts/demo-seed.sh. It never runs a model.
// Installed as claude, codex and pi, so herdr detects it as that agent by process name, and as demo-agent,
// which herdr does not detect (the seed script then reports the agent state itself).
// usage: <name> SCREEN [SCRIPT]
// Prints SCREEN (raw bytes; it may set the title with OSC 0) twice, 1.5 s apart, then waits for keys until the pane closes.
// Keys are not echoed. Without SCRIPT, keys are ignored.
//
// SCRIPT makes the pane answer keys (a permission prompt or a question picker). It is a list of blocks:
//   @ <state> <key>        the block runs when <key> arrives in <state>; the first state is "start"
//   show <file>            print a screen file
//   append <src> <dst>     append src to dst, with every @@NOW@@ replaced by the current UTC time (ISO 8601)
//   sleep <seconds>        decimal seconds
//   hold <file>            keep <file> open for reading; for a Claude log (~/.claude/projects/.../<id>.jsonl) also
//                          write ~/.claude/sessions/<pid>.json, the pid file that names a Claude session under herdr-core
//   state <name>           the state after this block (default: unchanged)
// Keys: a printable character, or ENTER, ESC, UP, DOWN, TAB. Keys that arrive while a block runs are dropped.
// A block for key INIT in state start runs once, before any key is read.
// Typed text: characters that match no block are kept as a line (Backspace removes one). ENTER with a non-empty
// line runs the block for key LINE in this state, if there is one; there, append also replaces @@TEXT@@ with the
// line, escaped for a JSON string. Any block that runs clears the line.
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

static void cat(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return;
    char buf[8192];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) fwrite(buf, 1, n, stdout);
    fclose(f);
    fflush(stdout);
}

// What Claude Code writes for herdr-core to find its session: the pid, the session id, and the start time as `ps lstart` in UTC.
static void claude_pid_file(const char *log) {
    const char *home = getenv("HOME"), *base = strrchr(log, '/');
    size_t n = base ? strlen(base + 1) : 0;
    if (!home || !strstr(log, "/.claude/projects/") || n < 7 || strcmp(base + 1 + n - 6, ".jsonl")) return;
    char cmd[64], start[128] = "", dir[2048], path[2100];
    snprintf(cmd, sizeof cmd, "TZ=UTC0 /bin/ps -o lstart= -p %d", (int)getpid());
    FILE *ps = popen(cmd, "r");
    if (!ps) return;
    if (!fgets(start, sizeof start, ps)) start[0] = 0;
    pclose(ps);
    start[strcspn(start, "\n")] = 0;
    snprintf(dir, sizeof dir, "%s/.claude", home);
    mkdir(dir, 0755);
    snprintf(dir, sizeof dir, "%s/.claude/sessions", home);
    mkdir(dir, 0755);
    snprintf(path, sizeof path, "%s/%d.json", dir, (int)getpid());
    FILE *f = fopen(path, "w");
    if (!f) return;
    fprintf(f, "{\"pid\":%d,\"sessionId\":\"%.*s\",\"procStart\":\"%s\"}\n", (int)getpid(), (int)(n - 6), base + 1, start);
    fclose(f);
}

static char typed[8192];
static size_t typed_len;

// The typed line as the inside of a JSON string.
static void put_json_text(FILE *out) {
    for (size_t i = 0; i < typed_len; i++) {
        unsigned char c = (unsigned char)typed[i];
        if (c == '"' || c == '\\') fprintf(out, "\\%c", c);
        else if (c == '\n') fputs("\\n", out);
        else if (c < 0x20) fprintf(out, "\\u%04x", c);
        else fputc(c, out);
    }
}

static void append(const char *src, const char *dst) {
    FILE *in = fopen(src, "rb");
    if (!in) return;
    static char buf[1 << 20];
    size_t n = fread(buf, 1, sizeof buf - 1, in);
    fclose(in);
    buf[n] = 0;
    struct timeval tv;
    gettimeofday(&tv, NULL);
    struct tm tm;
    gmtime_r(&tv.tv_sec, &tm);
    char now[32];
    snprintf(now, sizeof now, "%04d-%02d-%02dT%02d:%02d:%02d.%03dZ", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday,
             tm.tm_hour, tm.tm_min, tm.tm_sec, (int)(tv.tv_usec / 1000));
    FILE *out = fopen(dst, "ab");
    if (!out) return;
    const char *p = buf;
    for (;;) {
        const char *n = strstr(p, "@@NOW@@"), *t = strstr(p, "@@TEXT@@");
        const char *q = n && (!t || n < t) ? n : t;
        if (!q) break;
        fwrite(p, 1, q - p, out);
        if (q == n) { fputs(now, out); p = q + 7; }
        else { put_json_text(out); p = q + 8; }
    }
    fputs(p, out);
    fclose(out);
}

// Reads one key; returns its name in out ("1", "ENTER", "ESC", ...). Returns 0 on EOF.
static int read_key(char *out, size_t size) {
    unsigned char c;
    if (read(STDIN_FILENO, &c, 1) != 1) return 0;
    if (c == '\r' || c == '\n') snprintf(out, size, "ENTER");
    else if (c == '\t') snprintf(out, size, "TAB");
    else if (c == 0x7f || c == 0x08) snprintf(out, size, "BACKSPACE");
    else if (c == 0x1b) {
        struct pollfd p = {STDIN_FILENO, POLLIN, 0};
        unsigned char seq[2];
        if (poll(&p, 1, 40) > 0 && read(STDIN_FILENO, &seq[0], 1) == 1 && (seq[0] == '[' || seq[0] == 'O') &&
            read(STDIN_FILENO, &seq[1], 1) == 1) {
            // CSI with parameters (a bracketed paste mark, ESC [ 200 ~): read to its final byte.
            unsigned char f = seq[1];
            while (seq[0] == '[' && f >= 0x30 && f <= 0x3f && read(STDIN_FILENO, &f, 1) == 1) {}
            snprintf(out, size, seq[1] == 'A' ? "UP" : seq[1] == 'B' ? "DOWN" : "OTHER");
        } else snprintf(out, size, "ESC");
    } else snprintf(out, size, "%c", c);
    return 1;
}

static void drain(void) {
    struct pollfd p = {STDIN_FILENO, POLLIN, 0};
    unsigned char buf[256];
    while (poll(&p, 1, 0) > 0 && read(STDIN_FILENO, buf, sizeof buf) > 0) {}
}

// Runs the block for (state, key). Returns 1 if a block matched.
static int run(const char *script, char *state, const char *key) {
    FILE *f = fopen(script, "r");
    if (!f) return 0;
    char line[4096], a[2048], b[2048];
    int in = 0, matched = 0;
    while (fgets(line, sizeof line, f)) {
        line[strcspn(line, "\n")] = 0;
        if (line[0] == '@') {
            if (in) break;
            in = sscanf(line, "@ %2047s %2047s", a, b) == 2 && !strcmp(a, state) && !strcmp(b, key);
            matched |= in;
        } else if (!in) continue;
        else if (sscanf(line, "show %2047s", a) == 1) cat(a);
        else if (sscanf(line, "append %2047s %2047s", a, b) == 2) append(a, b);
        else if (sscanf(line, "hold %2047s", a) == 1) { if (fopen(a, "r") != NULL) claude_pid_file(a); }
        else if (sscanf(line, "sleep %2047s", a) == 1) usleep((useconds_t)(atof(a) * 1e6));
        else if (sscanf(line, "state %63s", a) == 1) snprintf(state, 64, "%s", a);
    }
    fclose(f);
    return matched;
}

int main(int argc, char **argv) {
    struct termios t;
    if (tcgetattr(STDIN_FILENO, &t) == 0) {
        t.c_lflag &= ~(ECHO | ICANON | ISIG);
        t.c_cc[VMIN] = 1;
        t.c_cc[VTIME] = 0;
        tcsetattr(STDIN_FILENO, TCSANOW, &t);
    }
    signal(SIGINT, SIG_IGN);
    if (argc > 1) {
        cat(argv[1]);
        usleep(1500000);   // draw again: herdr detects the agent on output, and the first draw can come too early
        cat(argv[1]);
    }
    if (argc < 3) for (;;) pause();
    char state[64] = "start", key[16];
    run(argv[2], state, "INIT");
    while (read_key(key, sizeof key)) {
        int ran = !strcmp(key, "ENTER") && typed_len && run(argv[2], state, "LINE");
        if (!ran) ran = run(argv[2], state, key);
        if (ran) { typed_len = 0; drain(); }
        else if (!strcmp(key, "BACKSPACE")) { if (typed_len) typed_len--; }
        else if (strlen(key) == 1 && typed_len < sizeof typed - 1) typed[typed_len++] = key[0];
    }
    for (;;) pause();
}
