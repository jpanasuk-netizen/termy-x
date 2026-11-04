/* Head-to-head VT throughput: libtermy vs libghostty-vt through their C ABIs.
 *
 * Usage: vt_library_bench [MiB] [case] [--frame]
 * Cases: all, plain, styled, unicode, varied-unicode, combining,
 * varied-combining, fragmented.
 *
 * Build the same source once per library:
 *   cc -O2 -DVT_BACKEND_TERMY -I crates/core/include vt_library_bench.c \
 *     -L target/release -ltermy_core -o bench-termy
 *   cc -O2 -DVT_BACKEND_GHOSTTY -I ghostty/include vt_library_bench.c \
 *     -L ghostty/zig-out/lib -lghostty-vt -o bench-ghostty
 *
 * Payloads, 120x40 grid, 1000-line history, 64 KiB blocks and 8 warmup blocks
 * match crates/core/examples/terminal_facade_bench.rs. `--frame` pulls a
 * renderer-ready update after every block (libtermy: take_frame_update with
 * dirty cells; libghostty: render_state_update + clean). No cells are drawn.
 * Run each case in its own process; scripts/benchmark-vt-libraries.py
 * alternates the two binaries and records medians.
 */

/* clock_gettime and ru_maxrss under -std=c11. */
#define _DEFAULT_SOURCE
#define _DARWIN_C_SOURCE

#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>

#define COLS 120
#define ROWS 40
#define HISTORY 1000
#define BLOCK (64 * 1024)

#if defined(VT_BACKEND_TERMY)
#include <termy.h>

typedef struct {
  TermyFfiTerminal *terminal;
} Vt;

static void check(TermyFfiStatus status, const char *what) {
  if (status != TERMY_FFI_OK) {
    fprintf(stderr, "%s failed: %d\n", what, (int)status);
    exit(1);
  }
}

static void vt_describe(void) { printf("engine: libtermy\n"); }

static Vt vt_new(void) {
  Vt vt = {0};
  TermyFfiSize size = termy_size_default();
  size.cols = COLS;
  size.rows = ROWS;
  check(termy_display_terminal_new(size, &vt.terminal), "termy_display_terminal_new");
  check(termy_terminal_set_scrollback_history(vt.terminal, HISTORY),
        "termy_terminal_set_scrollback_history");
  return vt;
}

static void vt_feed(Vt *vt, const uint8_t *bytes, size_t len) {
  check(termy_terminal_feed_output(vt->terminal, bytes, len), "termy_terminal_feed_output");
}

static size_t vt_frame(Vt *vt) {
  TermyFfiFrameUpdate update = {0};
  check(termy_terminal_take_frame_update(vt->terminal, false, &update),
        "termy_terminal_take_frame_update");
  size_t touched = update.cells_len + update.spans_len;
  check(termy_frame_update_free(&update), "termy_frame_update_free");
  return touched;
}

static void vt_free(Vt *vt) { check(termy_terminal_free(vt->terminal), "termy_terminal_free"); }

#elif defined(VT_BACKEND_GHOSTTY)
#include <ghostty/vt.h>

typedef struct {
  GhosttyTerminal terminal;
  GhosttyRenderState render;
} Vt;

static void check(GhosttyResult result, const char *what) {
  if (result != GHOSTTY_SUCCESS) {
    fprintf(stderr, "%s failed: %d\n", what, (int)result);
    exit(1);
  }
}

static void vt_describe(void) {
  static const char *modes[] = {"Debug", "ReleaseSafe", "ReleaseSmall", "ReleaseFast"};
  GhosttyString version = {0};
  GhosttyOptimizeMode optimize = GHOSTTY_OPTIMIZE_DEBUG;
  bool simd = false;
  check(ghostty_build_info(GHOSTTY_BUILD_INFO_VERSION_STRING, &version), "build_info version");
  check(ghostty_build_info(GHOSTTY_BUILD_INFO_OPTIMIZE, &optimize), "build_info optimize");
  check(ghostty_build_info(GHOSTTY_BUILD_INFO_SIMD, &simd), "build_info simd");
  printf("engine: libghostty-vt\n");
  printf("ghostty: %.*s, %s, simd=%s\n", (int)version.len, (const char *)version.ptr,
         (unsigned)optimize < 4 ? modes[optimize] : "unknown", simd ? "on" : "off");
}

static Vt vt_new(void) {
  Vt vt = {0};
  size_t history = HISTORY;
  check(ghostty_terminal_new(NULL, &vt.terminal, COLS, ROWS), "ghostty_terminal_new");
  check(ghostty_terminal_set(vt.terminal, GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES, &history),
        "ghostty_terminal_set scrollback");
  check(ghostty_render_state_new(NULL, &vt.render), "ghostty_render_state_new");
  return vt;
}

static void vt_feed(Vt *vt, const uint8_t *bytes, size_t len) {
  ghostty_terminal_vt_write(vt->terminal, bytes, len);
}

static size_t vt_frame(Vt *vt) {
  GhosttyRenderStateDirty dirty = GHOSTTY_RENDER_STATE_DIRTY_FALSE;
  check(ghostty_render_state_update(vt->render, vt->terminal), "ghostty_render_state_update");
  check(ghostty_render_state_get(vt->render, GHOSTTY_RENDER_STATE_DATA_DIRTY, &dirty),
        "ghostty_render_state_get dirty");
  check(ghostty_render_state_clean(vt->render), "ghostty_render_state_clean");
  return (size_t)dirty;
}

static void vt_free(Vt *vt) {
  ghostty_render_state_free(vt->render);
  ghostty_terminal_free(vt->terminal);
}

#else
#error "define VT_BACKEND_TERMY or VT_BACKEND_GHOSTTY"
#endif

typedef struct {
  uint8_t *ptr;
  size_t len;
  size_t cap;
} Buf;

static void buf_push(Buf *buf, const void *bytes, size_t len) {
  if (buf->len + len > buf->cap) {
    buf->cap = (buf->len + len) * 2;
    buf->ptr = realloc(buf->ptr, buf->cap);
    if (!buf->ptr) {
      perror("realloc");
      exit(1);
    }
  }
  memcpy(buf->ptr + buf->len, bytes, len);
  buf->len += len;
}

static void buf_push_utf8(Buf *buf, uint32_t scalar) {
  uint8_t out[4];
  size_t len;
  if (scalar < 0x80) {
    out[0] = (uint8_t)scalar;
    len = 1;
  } else if (scalar < 0x800) {
    out[0] = (uint8_t)(0xc0 | (scalar >> 6));
    out[1] = (uint8_t)(0x80 | (scalar & 0x3f));
    len = 2;
  } else if (scalar < 0x10000) {
    out[0] = (uint8_t)(0xe0 | (scalar >> 12));
    out[1] = (uint8_t)(0x80 | ((scalar >> 6) & 0x3f));
    out[2] = (uint8_t)(0x80 | (scalar & 0x3f));
    len = 3;
  } else {
    out[0] = (uint8_t)(0xf0 | (scalar >> 18));
    out[1] = (uint8_t)(0x80 | ((scalar >> 12) & 0x3f));
    out[2] = (uint8_t)(0x80 | ((scalar >> 6) & 0x3f));
    out[3] = (uint8_t)(0x80 | (scalar & 0x3f));
    len = 4;
  }
  buf_push(buf, out, len);
}

static double now_seconds(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static long peak_rss_kib(void) {
#ifdef __linux__
  /* ru_maxrss survives execve on Linux and would report the parent's peak;
   * VmHWM belongs to this process image only. */
  FILE *status = fopen("/proc/self/status", "r");
  char row[256];
  long kib = -1;
  while (status && fgets(row, sizeof(row), status)) {
    if (sscanf(row, "VmHWM: %ld kB", &kib) == 1) break;
  }
  if (status) fclose(status);
  return kib;
#else
  struct rusage usage;
  if (getrusage(RUSAGE_SELF, &usage) != 0) return -1;
  return usage.ru_maxrss / 1024;
#endif
}

/* Keeps the optimizer from discarding frame results. */
static volatile size_t sink;

static void run(const char *name, const Buf *line, size_t target, size_t fragmentation,
                bool frame) {
  Buf payload = {0};
  while (payload.len < BLOCK) buf_push(&payload, line->ptr, line->len);

  Vt vt = vt_new();
  for (int warmup = 0; warmup < 8; warmup++) {
    vt_feed(&vt, payload.ptr, payload.len);
    if (frame) sink += vt_frame(&vt);
  }

  size_t iterations = (target + payload.len - 1) / payload.len;
  double start = now_seconds();
  for (size_t i = 0; i < iterations; i++) {
    for (size_t offset = 0; offset < payload.len; offset += fragmentation) {
      size_t len = payload.len - offset < fragmentation ? payload.len - offset : fragmentation;
      vt_feed(&vt, payload.ptr + offset, len);
    }
    if (frame) sink += vt_frame(&vt);
  }
  double elapsed = now_seconds() - start;
  size_t bytes = iterations * payload.len;
  printf("%s: %.3f MiB/s; %zu bytes; %.6f seconds\n", name,
         (double)bytes / 1048576.0 / elapsed, bytes, elapsed);
  printf("peak rss: %ld KiB\n", peak_rss_kib());
  vt_free(&vt);
  free(payload.ptr);
}

static const char *const CASES[] = {"plain",     "styled",           "unicode",   "varied-unicode",
                                    "combining", "varied-combining", "fragmented"};

int main(int argc, char **argv) {
  long mib = 32;
  const char *selected = "all";
  bool frame = false;
  int positional = 0;
  for (int i = 1; i < argc; i++) {
    if (strcmp(argv[i], "--frame") == 0) {
      frame = true;
    } else if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
      printf("Usage: vt_library_bench [MiB] [case] [--frame]\n");
      return 0;
    } else if (argv[i][0] == '-') {
      fprintf(stderr, "unknown option: %s\n", argv[i]);
      return 2;
    } else if (positional == 0) {
      char *end = NULL;
      mib = strtol(argv[i], &end, 10);
      if (*end != '\0' || mib < 1 || mib > 4096) {
        fprintf(stderr, "MiB must be an integer in 1..=4096\n");
        return 2;
      }
      positional++;
    } else if (positional == 1) {
      selected = argv[i];
      positional++;
    } else {
      fprintf(stderr, "expected at most MiB and case\n");
      return 2;
    }
  }
  bool known = strcmp(selected, "all") == 0;
  for (size_t i = 0; i < sizeof(CASES) / sizeof(CASES[0]); i++) {
    known = known || strcmp(selected, CASES[i]) == 0;
  }
  if (!known) {
    fprintf(stderr, "unknown case: %s\n", selected);
    return 2;
  }

  vt_describe();
  printf("vt_library_bench: %dx%d, %d history lines, %ld MiB per case, frame=%s\n", COLS, ROWS,
         HISTORY, mib, frame ? "true" : "false");
  size_t target = (size_t)mib * 1048576;

#define WANT(name) (strcmp(selected, "all") == 0 || strcmp(selected, (name)) == 0)
  Buf line = {0};
  const char *plain = "the quick brown fox jumps over the lazy dog 0123456789\r\n";
  if (WANT("plain")) {
    line.len = 0;
    buf_push(&line, plain, strlen(plain));
    run("plain", &line, target, BLOCK, frame);
  }
  if (WANT("styled")) {
    const char *styled = "\x1b[H\x1b[38;2;120;180;255;48;5;234;1mstatus\x1b[0m\x1b[2;1H\x1b[Kbuild "
                         "complete\x1b[3;1Hprogress: 100%";
    line.len = 0;
    buf_push(&line, styled, strlen(styled));
    run("styled", &line, target, BLOCK, frame);
  }
  if (WANT("unicode")) {
    const char *unicode = "ASCII caf\xc3\xa9 \xce\x95\xce\xbb\xce\xbb\xce\xb7\xce\xbd\xce\xb9\xce"
                          "\xba\xce\xac \xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e \xed\x95\x9c\xea\xb8"
                          "\x80 \xf0\x9f\x99\x82\xe7\x95\x8c\r\n";
    line.len = 0;
    buf_push(&line, unicode, strlen(unicode));
    run("unicode", &line, target, BLOCK, frame);
  }
  if (WANT("varied-unicode")) {
    line.len = 0;
    for (uint32_t scalar = 0x4e00; scalar < 0x8e00; scalar++) {
      buf_push_utf8(&line, scalar);
      if (scalar % 40 == 39) buf_push(&line, "\r\n", 2);
    }
    run("varied-unicode", &line, target, BLOCK, frame);
  }
  if (WANT("combining")) {
    const char *combining = "e\xcc\x81 a\xcc\x88 n\xcc\x83 o\xcc\x82 u\xcc\x8a\r\n";
    line.len = 0;
    buf_push(&line, combining, strlen(combining));
    run("combining", &line, target, BLOCK, frame);
  }
  if (WANT("varied-combining")) {
    line.len = 0;
    for (uint32_t scalar = 0x0300, index = 0; scalar <= 0x036f; scalar++, index++) {
      buf_push(&line, "e", 1);
      buf_push_utf8(&line, scalar);
      buf_push(&line, " ", 1);
      if (index % 10 == 9) buf_push(&line, "\r\n", 2);
    }
    buf_push(&line, "\r\n", 2);
    run("varied-combining", &line, target, BLOCK, frame);
  }
  if (WANT("fragmented")) {
    line.len = 0;
    buf_push(&line, plain, strlen(plain));
    run("fragmented", &line, target, 1, frame);
  }
#undef WANT
  free(line.ptr);
  return 0;
}
