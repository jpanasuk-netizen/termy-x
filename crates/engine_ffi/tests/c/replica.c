/* A server terminal answers queries; a replica restored from its snapshot
 * mirrors the same output silently and ends in the same state. */
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#include "termy_engine.h"

#define CHECK(condition)                                                    \
  do {                                                                      \
    if (!(condition)) {                                                     \
      fprintf(stderr, "%s:%d: check failed: %s\n", __FILE__, __LINE__,      \
              #condition);                                                  \
      return 1;                                                             \
    }                                                                       \
  } while (0)

static int feed(TermyEngine *engine, const char *text, TermyEngineBytes *replies) {
  return termy_engine_feed(engine, (const uint8_t *)text, strlen(text), replies);
}

int main(void) {
  CHECK(termy_engine_abi_version() == TERMY_ENGINE_ABI_VERSION);
  /* Layouts mirrored by const assertions in src/frame.rs and src/input.rs. */
  CHECK(sizeof(TermyEngineCell) == 32);
  CHECK(offsetof(TermyEngineCell, grapheme_start) == 4);
  CHECK(offsetof(TermyEngineCell, grapheme_len) == 8);
  CHECK(offsetof(TermyEngineCell, width) == 10);
  CHECK(offsetof(TermyEngineCell, underline) == 11);
  CHECK(offsetof(TermyEngineCell, foreground) == 12);
  CHECK(offsetof(TermyEngineCell, background) == 16);
  CHECK(offsetof(TermyEngineCell, underline_color) == 20);
  CHECK(offsetof(TermyEngineCell, flags) == 24);
  CHECK(offsetof(TermyEngineCell, hyperlink) == 28);
  CHECK(offsetof(TermyEngineKeyEvent, unshifted_codepoint) == 4 * sizeof(void *));
  CHECK(offsetof(TermyEngineKeyEvent, action) == 4 * sizeof(void *) + 12);
  CHECK(offsetof(TermyEngineKeyEvent, composing) == 4 * sizeof(void *) + 13);
  CHECK(offsetof(TermyEngineMouseEvent, any_button_pressed) == 2);
  CHECK(offsetof(TermyEngineMouseEvent, mods) == 4);
  CHECK(offsetof(TermyEngineMouseEvent, y_px) == 20);
  CHECK(sizeof(TermyEngineMouseEvent) == 24);

  TermyEngineOptions options = termy_engine_options_default();
  options.cols = 40;
  options.rows = 10;
  TermyEngine *server = NULL;
  CHECK(termy_engine_new(&options, &server) == TERMY_ENGINE_OK);

  TermyEngineBytes replies = {0};
  CHECK(feed(server, "\x1b[1mhello\x1b[0m \xe4\xb8\xad\r\n\x1b]2;title\x07\x1b[", &replies) ==
        TERMY_ENGINE_OK);
  CHECK(replies.len == 0);

  TermyEngineBytes snapshot = {0};
  CHECK(termy_engine_snapshot_encode(server, &snapshot) == TERMY_ENGINE_OK);
  uint32_t version = 0;
  CHECK(termy_engine_snapshot_peek_version(snapshot.ptr, snapshot.len, &version) ==
        TERMY_ENGINE_OK);
  CHECK(version == termy_engine_snapshot_version());

  TermyEngine *replica = NULL;
  CHECK(termy_engine_snapshot_decode(snapshot.ptr, snapshot.len,
                                     TERMY_ENGINE_REPLIES_DISCARD,
                                     &replica) == TERMY_ENGINE_OK);
  termy_engine_bytes_free(&snapshot);
  CHECK(snapshot.ptr == NULL);

  /* The snapshot ended mid-CSI: "5n" completes a status query. */
  const char *tail = "5n\x1b[c world";
  CHECK(feed(server, tail, &replies) == TERMY_ENGINE_OK);
  CHECK(replies.len > 4 && memcmp(replies.ptr, "\x1b[0n", 4) == 0);
  termy_engine_bytes_free(&replies);
  CHECK(feed(replica, tail, &replies) == TERMY_ENGINE_OK);
  CHECK(replies.len == 0);

  TermyEngineBytes a = {0};
  TermyEngineBytes b = {0};
  CHECK(termy_engine_snapshot_encode(server, &a) == TERMY_ENGINE_OK);
  CHECK(termy_engine_snapshot_encode(replica, &b) == TERMY_ENGINE_OK);
  CHECK(a.len == b.len && memcmp(a.ptr, b.ptr, a.len) == 0);
  termy_engine_bytes_free(&a);
  termy_engine_bytes_free(&b);

  TermyEngineFrame frame;
  CHECK(termy_engine_frame(replica, false, &frame) == TERMY_ENGINE_OK);
  CHECK(frame.damage == TERMY_ENGINE_DAMAGE_FULL);
  CHECK(frame.cells_len == 400);
  CHECK(frame.cells_ptr[0].codepoint == 'h');
  CHECK(frame.cells_ptr[0].flags & TERMY_ENGINE_CELL_BOLD);
  CHECK(frame.cells_ptr[6].width == TERMY_ENGINE_CELL_WIDE);
  CHECK(frame.cells_ptr[7].width == TERMY_ENGINE_CELL_SPACER_TAIL);

  TermyEngineBytes title = {0};
  CHECK(termy_engine_title(replica, &title) == TERMY_ENGINE_OK);
  CHECK(title.len == 5 && memcmp(title.ptr, "title", 5) == 0);
  termy_engine_bytes_free(&title);

  uint8_t report = 0;
  CHECK(termy_engine_mode(replica, 2027, false, &report) == TERMY_ENGINE_OK);
  CHECK(report == 3);

  TermyEngineBytes text = {0};
  CHECK(termy_engine_read_text(replica, NULL, TERMY_ENGINE_FORMAT_PLAIN,
                               TERMY_ENGINE_TEXT_TRIM, &text) == TERMY_ENGINE_OK);
  CHECK(text.len >= 9 && memcmp(text.ptr, "hello \xe4\xb8\xad\n", 10) == 0);
  termy_engine_bytes_free(&text);

  TermyEngineBytes paste = {0};
  CHECK(termy_engine_encode_paste((const uint8_t *)"a\nb", 3, true, &paste) == TERMY_ENGINE_OK);
  CHECK(paste.len == 15 && memcmp(paste.ptr, "\x1b[200~a\rb\x1b[201~", 15) == 0);
  termy_engine_bytes_free(&paste);

  termy_engine_free(replica);
  termy_engine_free(server);
  puts("ok");
  return 0;
}
