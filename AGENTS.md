# Anchored Summary — Rustbox WM + dunst-reimplementation (notifications)

## Goal
- Finish the Rustbox WM audit (16/16 done; #14 re-scoped as a dunst reimplementation) and rebuild the **dunst notification daemon in Rust, integrated into the WM**, with lightweight rendering (x11rb + TrueType/emoji), faithful/complete feature set (markup, rules, icons, dunstrc-like config, dunstctl).

## Constraints & Preferences
- No root on Termux; Termux fork separate — PC not limited by Termux.
- X11 socket: `/tmp/.X11-unix/X1` (Xephyr `:1`); `:0` is Xwayland.
- Session D-Bus: `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`.
- Render: x11rb + TrueType/anti-aliased (`fontdb` + `ab_glyph`) + emoji (`ttf_parser` PNG); **no cairo/pango**; `bitmap_font` as fallback.
- Scope: notificação + WM integrado; X11 only (Wayland out).
- `libc::poll` no loop; reportar em português.

## Progress

### Audit (16/16)
#1–#16 concluídos + #14 reescopo → dunst em Rust.

### Notificação — todos os recursos implementados
- **D-Bus org.freedesktop.Notifications**: queue, timeout, pause, signals, `GetCapabilities` honesto (body, actions, icon-static, persistence, image/png, urgency, x-canonical-private-synchronous, x-dunst-stack-tag).
- **dunstctl org.dunstproject.cmd0**: ping/pause/resume/isPaused/close/closeAll/history/getHistory/context/closeLast/historyCount/clearHistory/removeFromHistory/popHistory/configReload.
- **Regras**: regex (appname/summary/body/urgency/category/stack_tag/desktop_entry), override (timeout/urgency/icon), skip_display, set_transient, history_ignore.
- **Config dunstrc-like**: `[global]` (width/margin/gap/max_visible/timeout/monitor/font_scale/origin/bg/fg/frame/body/urgency_colors) + secões de regra. SIGUSR1 reload.
- **Ícones**: image-path, image-data (RGBA8), desktop-entry → theme icon lookup (XDG_DATA_DIRS/usr/share/~/.local/share/~/.icons/pixmaps), sync_map para replace síncrono, ImageControl::load_memory.
- **Stack_tag**: substituição de notificações com mesma stack_tag (como sync_key).
- **Barra de progresso**: hint `value` 0–100 desenhada no Popup (track + fill + %).
- **Tema de cores**: Theme (bg/fg/frame/body/urgency[3] RGB) aplicado a Popup (fundo, borda, barra de urgência, texto via fg_pixel).

### Sistema de fontes (novo) — `src/render/font.rs`
- **Descoberta automática**: `fontdb::Database` (lazy static) carrega fontes do sistema.
- **Rasterização anti-aliased**: `ab_glyph` (FontRef + outline_glyph + draw) para texto grayscale.
- **PutImage BGRX**: 4 bytes/pixel, mesmo formato de image.rs.
- **Emoji**: `ttf_parser::Face::glyph_raster_image` extrai PNGs do Noto Color Emoji (CBDT/CBLC); alpha-blend no buffer.
- **Fallback**: `bitmap_font` quando sem dados TrueType.
- **Cache**: `RefCell<HashMap<u32, Glyph>>` para glyphs rasterizados.
- **Word wrap**: `wrap(text, max_width)` por contagem de caracteres (aproximado).

### Integração do Font em todos os consumidores
- `draw_text(conn, drawable, gc, x, y, text, fg)` — assinatura de **7 parâmetros** (fg é pixel X11).
- `Popup` (notificações): substituiu bitmap_font direto.
- `window/frame.rs`: título das janelas.
- `toolbar/mod.rs`: relógio + workspaces.
- `menu/menu.rs`: itens de menu + setas.
- `screen/mod.rs`: diálogos.
- `bin/fbrun.rs`: input do run dialog.

### Testes
**9 passam** (0 failed):
- 5 em `notify/tests`
- 3 em `render/font/tests` (emoji_detection, create_gets_a_font, measure_something)
- 1 em `menu/parser/tests`

### Build
`cargo build` completo ✅ (lib + bin), 0 erros, 3 warnings (dead code etc.).

## Key Decisions
- dunst em Rust integrado; TrueType/emoji (fontdb + ab_glyph + ttf_parser) em vez de bitmap.
- `dbus 0.9 ffidisp` (non-blocking; `register_object_path` obrigatório; Connection não-Send).
- Regras com `regex` (sem lock); parser de config manual (sem configparser/ini/toml).
- `catch_unwind` em process_dbus+tick (event/mod.rs:411).
- `draw_text` com `fg_pixel` explícito (PutImage não usa GC foreground).
- SVG ignorado (image crate não decodifica).

## Next Steps
1. **Wrap pixel-perfeito**: substituir `wrap` baseado em chars por `measure()` (precisa de `text_width` por fragmento).
2. **COLR/CPAL**: suporte para fontes de emoji baseadas em layered color glyphs (COLRv0/v1), não só PNG embutido.
3. **dunstctl restantes**: RuleEnable, RuleList, ContextMenuCall, NotificationAction, Show; props D-Bus paused/pauseLevel.
4. **Regras faltantes**: format, script, fullscreen, override_pause_level, min/max_icon_size, alignment, icon_position.
5. **Propriedades paused/pauseLevel** como D-Bus properties (rw).
6. **Teste ao vivo**: `timeout 3 ./target/debug/rustbox` no Xephyr `:1` + `notify-send -t 2000 "teste"`.
7. **bitmap_font removido** se fallback nunca usado (deferido).

## Critical Context
- `Font::draw_text(conn, drawable, gc, x, y, text, fg)` — 7 args, BGRX PutImage, background branco.
- `render_emoji(codepoint: u32)` → `Option<(w, h, RGBA8 raw)>` via ttf_parser + image crate.
- `is_emoji(cp)` — ranges Unicode de emoji (inclui ZWJ `0x200D`).
- `fontdb 0.21`, `ab_glyph 0.2`, `ttf-parser 0.25`. Traits `Font as _` e `ScaleFont as _` em escopo.
- `resolve_font_bytes(family)` → fontdb Query → `with_face_data`.
- `EmojiFont` lazy via `OnceLock<Mutex<...>>`, paths: `/usr/share/fonts/noto/NotoColorEmoji.ttf` etc.
- `Config` defaults: max_visible=5, width=380, margin=12, gap=8, default_timeout_ms=5000, origin=TopRight, monitor=0, font_scale=2, rules=[].
- `RawNotification`: id, app_name, app_icon, summary, body, actions, urgency, transient, expire_timeout, created, icon_data, sync_key, stack_tag, category, desktop_entry, progress, history_ignore.
- `Rule`: appname_re, summary_re, body_re, urgency, expire_timeout, set_urgency, new_icon, skip_display, category_re, stack_tag_re, desktop_entry_re, set_transient, history_ignore.
- Harness limitation: background WM killed → use `cargo test` + `timeout 3 ./target/debug/rustbox`.

## Relevant Files
- `src/notify/mod.rs` — daemon, D-Bus, regras, config, hints, sync_map, dunstctl.
- `src/notify/render.rs` — Popup (TrueType, ícones, barra de progresso, ações, tema).
- `src/render/font.rs` — **sistema de fontes**: fontdb, ab_glyph, emoji, PutImage, fallback.
- `src/render/bitmap_font.rs` — fallback (8×8) para quando não há TrueType.
- `src/render/image.rs` — Image/ImageControl (load/scale/create_pixmap).
- `src/event/mod.rs` — poll loop + catch_unwind.
- `src/window/frame.rs` — título com `draw_text(fg)`.
- `src/toolbar/mod.rs` — relógio/workspaces com `draw_text(fg)`.
- `src/menu/menu.rs` — itens + setas com `draw_text(fg)`.
- `src/screen/mod.rs` — diálogo com `draw_text(fg)`.
- `src/bin/fbrun.rs` — input com `draw_text(fg)`.
- `Cargo.toml` — deps: dbus="0.9", image="0.25", regex="1", serde, ttf-parser, fontdb="0.21", ab_glyph="0.2".
- `~/.config/rustbox/notifications.conf` — exemplo config.
