# Rustbox-rs — Análise Técnica de Bugs, Performance e Compatibilidade

**Data:** 11 de Julho de 2026  
**Versão analisada:** 1.4.0  
**Linhas de código:** ~9.436 (38 arquivos Rust)

---

## 🚨 Bugs Críticos (Corretivos Imediatos)

### 1. **Fechamento de Janelas: Graceful Shutdown Não Funciona**
**Local:** `src/screen/mod.rs:660-673` (`close_window`)

**Problema:**
```rust
// Envia WM_DELETE_WINDOW (pedido gentil)
let ev = ClientMessageEvent::new(32, window, wm_protocols, data);
conn.send_event(false, window, EventMask::NO_EVENT, &ev)?;

// IMEDIATAMENTE mata a conexão (sem esperar resposta!)
conn.kill_client(window)?;  // ❌ BUG: fecha antes do app processar
```

**Impacto:**
- Editores de texto (VSCode, Sublime, gedit) **não salvam** antes de fechar
- IDEs (IntelliJ, Eclipse) perdem estado de sessão
- Apps pedem "Salvar alterações?" mas são mortos antes de responder
- **Violação ICCCM**: `WM_DELETE_WINDOW` é um protocolo de *dois estágios*, não um kill imediato

**Solução Correta:**
```rust
// 1. Verificar se o cliente suporta WM_DELETE_WINDOW
let protocols = get_wm_protocols(window)?;
if protocols.contains(&WM_DELETE_WINDOW) {
    // Enviar e AGUARDAR (timeout 5s)
    send_delete_message(window)?;
    schedule_force_kill_if_not_closed(window, timeout=5000)?;
} else {
    // Cliente não suporta → kill imediato (fallback)
    conn.kill_client(window)?;
}
```

**Prioridade:** 🔴 **CRÍTICA** (perda de dados do usuário)

---

### 2. **Diálogo de Renomear Workspace: Não Aceita Input de Teclado**
**Local:** `src/screen/mod.rs:1536-1600` (`show_rename_dialog`, `handle_dialog_key`)

**Sintoma:**
- Diálogo aparece centralizado, renderiza corretamente
- **Nenhuma tecla digitada é processada** (texto não aparece)
- Clique do mouse funciona (fecha diálogo), mas teclado não

**Causa Raiz (Hipótese Forte):**
1. **SYNC Pointer Grab não liberado:** `manage_window()` faz `grab_button(..., pointer_mode: SYNC)` em todas as janelas gerenciadas, incluindo o diálogo
2. **Falta `allow_events()`:** Após `set_input_focus()` no diálogo, o pointer permanece congelado até um `ButtonPress` ser processado
3. **Foco perdido:** Se o mouse estiver sobre outra janela, `EnterNotify` pode redirecionar o foco antes do usuário digitar

**Código Problemático:**
```rust
// manage_window() linha ~580
conn.grab_button(..., pointer_mode: SYNC, ...)?;  // ❌ Congela pointer

// create_dialog() linha ~1580
conn.set_input_focus(..., dialog_client, ...)?;   // ✅ Foco definido
// ❌ MAS: pointer ainda está SYNC-grabbed!
// ❌ Falta: conn.allow_events(AllowMode::ASYNC_POINTER, time)?;
```

**Solução:**
```rust
// Após set_input_focus no diálogo:
conn.allow_events(AllowMode::ASYNC_POINTER, CURRENT_TIME)?;

// OU: usar pointer_mode: ASYNC no grab_button original
conn.grab_button(..., pointer_mode: ASYNC, ...)?;  // ✅ Não congela
```

**Prioridade:** 🔴 **CRÍTICA** (feature quebrada)

---

### 3. **Vazamento de Recursos X11 (Pixmaps, GCs, Fontes)**
**Local:** Múltiplos arquivos

**Contagem de Recursos:**
| Recurso | Criações | Liberações | Vazamento |
|---------|----------|------------|-----------|
| `create_pixmap` | 4 | 1 | **3** |
| `create_gc` | 11 | 9 | **2** |
| `open_font` | 5 | 3 | **2** |
| `create_glyph_cursor` | 3 | 1 | **2** |
| `create_window` | 11 | 10 | **1** |

**Locais de Vazamento:**
- `src/render/texture.rs:113` → `create_pixmap` sem `free_pixmap` correspondente
- `src/render/image.rs:33` → `Image::create_pixmap` retorna raw ID, caller nunca libera
- `src/window/frame.rs` → GCs e fonts criados no construtor, mas `destroy()` não libera todos

**Impacto:**
- **Memory leak no X server**: Após horas/dias de uso, X11 esgota IDs de recursos
- **Degradação gradual**: Rendering fica mais lento, eventualmente falha com `BadAlloc`
- **Crash do X server** em sessões longas (comum em mobile/Termux)

**Solução:**
```rust
// 1. Implementar Drop para Pixmap wrapper
impl Drop for Pixmap {
    fn drop(&mut self) {
        let _ = self.conn.free_pixmap(self.id);
    }
}

// 2. Auditoria manual: garantir que TODO create_* tenha free_* correspondente
// 3. Usar RAII pattern para GCs e Fonts
```

**Prioridade:** 🟠 **ALTA** (estabilidade de longo prazo)

---

### 4. **WM_TAKE_FOCUS Nunca Enviado (Quebra de Compatibilidade ICCCM)**
**Local:** `src/x11/atoms.rs` (átomo definido mas **nunca usado**)

**Problema:**
- `WmTakeFocus` está definido no enum de átomos
- **Nenhum código envia `WM_TAKE_FOCUS`** para clientes
- `manage_window()` foca janelas incondicionalmente via `set_input_focus()`

**Impacto (Aplicativos Afetados):**
- **Java/Swing**: IntelliJ IDEA, Android Studio, Minecraft → foco não funciona corretamente
- **GTK3/4**: Apps com input model "locally active" → teclado não responde
- **Qt**: Alguns apps não recebem foco até clique manual
- **ICCCM "Globally Active" windows**: Ignoradas completamente

**Solução:**
```rust
// Em manage_window(), após verificar WM_HINTS.input:
if client_wants_focus && supports_wm_take_focus(window) {
    // Enviar WM_TAKE_FOCUS (cliente decide quando focar)
    let ev = ClientMessageEvent::new(32, window, wm_protocols, [WM_TAKE_FOCUS, timestamp, ...]);
    conn.send_event(false, window, NO_EVENT, &ev)?;
} else {
    // Foco direto (para clientes "NoInput" ou "Passive")
    conn.set_input_focus(INPUT_FOCUS_POINTER_ROOT, window, CURRENT_TIME)?;
}
```

**Prioridade:** 🟠 **ALTA** (compatibilidade com apps populares)

---

## ⚠️ Problemas de Performance

### 1. **Event Loop: Busy-Wait de 10ms (100Hz) — Drenagem de Bateria**
**Local:** `src/event/mod.rs:80-95`

**Código Atual:**
```rust
loop {
    // Polling ativo: acorda a cada 10ms mesmo sem eventos
    while let Some(event) = conn.poll_for_event()? {
        self.handle_event(event)?;
    }
    
    std::thread::sleep(Duration::from_millis(10));  // ❌ 100 wakeups/segundo
    tick_counter += 1;
    
    if tick_counter % 100 == 0 {
        // Atualizar relógio a cada 1s
        self.update_clock()?;
    }
}
```

**Impacto:**
- **CPU usage mínimo 1-3%** mesmo idle (100 wakeups/s × processamento X11)
- **Bateria drenada** em mobile (Termux/Android)
- **Aquecimento** desnecessário

**Solução Ótima (Blocking Wait com Timeout):**
```rust
use polling::{Poller, Event as PollEvent};
use std::os::unix::io::AsRawFd;

let poller = Poller::new()?;
let x11_fd = conn.as_raw_fd();

loop {
    // Calcular timeout até próximo tick de relógio
    let timeout_ms = compute_time_until_next_clock_tick();
    
    // Adicionar X11 FD ao poller
    poller.add(&x11_fd, PollEvent::READABLE)?;
    
    // Bloquear até evento OU timeout
    let mut events = Vec::new();
    poller.wait(&mut events, Duration::from_millis(timeout_ms))?;
    
    if events.iter().any(|e| e.readable) {
        // Processar TODOS eventos X11 pendentes (drain)
        while let Some(event) = conn.poll_for_event()? {
            self.handle_event(event)?;
        }
    }
    
    // Atualizar relógio se timeout expirou
    if clock_tick_due() {
        self.update_clock()?;
    }
}
```

**Benefícios:**
- **0% CPU em idle** (processo dorme profundamente)
- **Bateria preservada** (crítico para mobile)
- **Latência mantida** (eventos X11 ainda processados instantaneamente)

**Prioridade:** 🟡 **MÉDIA** (performance/bateria)

---

### 2. **Crate `polling` Importada mas Não Utilizada**
**Local:** `Cargo.toml` (dependência presente), `src/` (nenhum uso)

**Problema:**
```toml
[dependencies]
polling = "3.0"  # ✅ Importada
# ... mas nenhum "use polling" ou Poller::new() em todo o código
```

**Ação:**
- **Opção A:** Implementar blocking wait (ver solução acima) → usar `polling`
- **Opção B:** Remover dependência → reduzir binário em ~50KB

**Prioridade:** 🟢 **BAIXA** (limpeza)

---

## 🧩 Compatibilidade com Aplicações

### 1. **Java/Swing (IntelliJ, Minecraft, NetBeans)**
**Problemas Combinados:**
- ❌ `WM_TAKE_FOCUS` não enviado (ver bug #4)
- ❌ `WM_HINTS.input` não verificado (foco forçado em janelas "NoInput")
- ⚠️ SYNC pointer grab pode travar input se `allow_events()` falhar

**Sintomas:**
- Teclado não responde até clique manual
- Foco "pula" entre janelas aleatoriamente
- Menus dropdown não fecham com clique externo

**Solução:** Ver bugs #2 e #4

---

### 2. **Electron/Chromium (VSCode, Discord, Brave)**
**Problemas Potenciais:**
- ⚠️ `_NET_WM_STATE_FULLSCREEN` implementado, mas `_NET_WM_STATE_MAXIMIZED_*` pode não ser respeitado
- ⚠️ `WM_DELETE_WINDOW` kill imediato → VSCode não salva session state
- ⚠️ Ozone/Wayland hijack já mitigado (`remove_var("WAYLAND_DISPLAY")`)

**Sintomas:**
- VSCode perde estado de workspace ao fechar
- Discord não minimiza para tray corretamente
- Brave abre em display errado se `XDG_SESSION_TYPE` não for zerado

**Solução:** Bug #1 (graceful close) + já mitigado (env vars)

---

### 3. **GTK3/4 (Gedit, Nautilus, GIMP)**
**Problemas:**
- ❌ `WM_TAKE_FOCUS` não enviado (GTK usa input model "locally active")
- ⚠️ `_NET_WM_STATE_SKIP_TASKBAR` parsing implementado, mas aplicação pode falhar

**Sintomas:**
- Gedit não foca até clique manual
- Nautilus abre múltiplas instâncias (foco não detectado)
- GIMP: ferramentas flutuantes não recebem foco

**Solução:** Bug #4

---

### 4. **Qt (VLC, Calibre, VirtualBox)**
**Problemas:**
- ⚠️ Qt respeita `WM_DELETE_WINDOW`, mas kill imediato quebra
- ⚠️ `_NET_WM_STATE_ABOVE/BELOW` pode não ser aplicado corretamente

**Sintomas:**
- VLC não salva playlist ao fechar
- Calibre: diálogo de metadados não foca
- VirtualBox: janela guest não entra em fullscreen

**Solução:** Bug #1 + verificar `_NET_WM_STATE` handling

---

## 🏗️ Architecture & Code Quality

### 1. **`panic = "abort"` em Release — Crash sem Recuperação**
**Local:** `Cargo.toml`
```toml
[profile.release]
panic = "abort"  # ❌ Sem unwind, sem catch_unwind possível
```

**Impacto:**
- Qualquer `unwrap()` ou `panic!` → **processo morto instantaneamente**
- **Janelas órfãs** na tela (apps ainda rodando, mas sem WM)
- **Sem graceful shutdown** (X11 não limpa recursos)

**Mitigação Parcial:**
- Panic hook grava em `~/.rustbox/panic.log` (útil para debug)
- Apenas **2 `unwrap()`** no código base (ambos em `screen/mod.rs`, ambos protegidos por `if is_some()`)

**Recomendação:**
```toml
# Manter panic = "abort" (performance, binário menor)
# MAS: aumentar auditoria de unwrap/expect
# E: adicionar fallback para erros fatais (ex: reconnect X11 se possível)
```

**Prioridade:** 🟢 **BAIXA** (já mitigado com poucos unwraps)

---

### 2. **101 Instâncias de `let _ =` — Erros Silenciosamente Descartados**
**Distribuição:**
- `src/screen/mod.rs`: 72
- `src/tray/mod.rs`: 14
- `src/window/frame.rs`: 5
- Outros: 10

**Exemplos Críticos:**
```rust
let _ = conn.set_input_focus(...);  // ❌ Falha sem log
let _ = conn.grab_pointer(...);     // ❌ Grab falha, input quebrado
let _ = conn.reparent_window(...);  // ❌ Reparent falha, janela some
```

**Impacto:**
- **Debug impossível**: falhas não aparecem em logs
- **Estado inconsistente**: WM acha que operação succeeded, mas X11 falhou
- **Bugs intermitentes**: "funciona às vezes" (depende de timing do X server)

**Solução:**
```rust
// Padrão recomendado:
match conn.set_input_focus(...) {
    Ok(_) => log::debug!("Focus set"),
    Err(e) => log::warn!("set_input_focus failed: {}", e),
}

// OU (se erro for realmente ignorável):
if let Err(e) = conn.set_input_focus(...) {
    log::debug!("set_input_focus ignored: {}", e);
}
```

**Prioridade:** 🟡 **MÉDIA** (observabilidade/debugabilidade)

---

### 3. **`reinit_key_grabs()` Definida mas Nunca Chamada (Dead Code)**
**Local:** `src/screen/mod.rs:1638`

**Código:**
```rust
/// Reinitialize key grabs after dialog close (ungrab_all clears them).
fn reinit_key_grabs(&self) -> Result<(), anyhow::Error> {
    // ... código para re-grabar todas as teclas ...
}
```

**Problema:**
- Função **nunca é chamada** em nenhum lugar
- `close_dialog()` não chama `reinit_key_grabs()`
- Comentário menciona `ungrab_all` que **não existe**

**Impacto:**
- Se `unmanage_window()` ou outro código limpar grabs, **hotkeys param de funcionar** após diálogo
- **Silent failure**: usuário aperta Mod+Tab, nada acontece

**Ação:**
- **Opção A:** Remover função dead code
- **Opção B:** Implementar `ungrab_all()` e chamar em `close_dialog()` + `reinit_key_grabs()` após

**Prioridade:** 🟢 **BAIXA** (dead code, não afeta runtime atual)

---

### 4. **`ImageControl` Cache com LRU Bugado**
**Local:** `src/render/image.rs:85-105`

**Código:**
```rust
if self.cache.len() >= self.max_size {
    if let Some(key) = self.cache.keys().next().cloned() {
        self.cache.remove(&key);  // ❌ HashMap ordem arbitrária!
    }
}
```

**Problema:**
- `HashMap::keys().next()` retorna **chave aleatória** (ordem não garantida)
- **LRU vira "random eviction"**: imagem errada é removida
- **Cache thrashing**: imagens usadas recentemente podem ser ejetadas

**Solução:**
```rust
use indexmap::IndexMap;  // Crate indexmap (ordem de inserção preservada)

pub struct ImageControl {
    cache: IndexMap<String, Arc<Image>>,  // ✅ FIFO/LRU correto
    max_size: usize,
}
```

**Prioridade:** 🟢 **BAIXA** (feature de imagem não usada atualmente)

---

## 📋 Resumo de Prioridades

| Prioridade | Bug | Impacto | Esforço |
|------------|-----|---------|---------|
| 🔴 CRÍTICA | #1 Graceful close | Perda de dados | 2-3h |
| 🔴 CRÍTICA | #2 Dialog input | Feature quebrada | 1h |
| 🟠 ALTA | #3 Resource leak | Crash após horas | 4-6h |
| 🟠 ALTA | #4 WM_TAKE_FOCUS | Apps Java/GTK quebrados | 2-3h |
| 🟡 MÉDIA | Event loop busy-wait | Bateria/CPU | 3-4h |
| 🟡 MÉDIA | Silent errors (`let _ =`) | Debug difícil | 2-3h |
| 🟢 BAIXA | panic = "abort" | Crash sem unwind | 0h (já mitigado) |
| 🟢 BAIXA | Dead code | Limpeza | 30min |

---

## ✅ Próximos Passos Recomendados

1. **Corrigir Bug #1 (Graceful Close)** — Evitar perda de dados do usuário
2. **Corrigir Bug #2 (Dialog Input)** — Habilitar feature de renomear workspace
3. **Corrigir Bug #4 (WM_TAKE_FOCUS)** — Compatibilidade com Java/GTK
4. **Auditoria de Recursos** — Garantir TODO `create_*` tem `free_*`
5. **Refatorar Event Loop** — Usar blocking wait com `polling`
6. **Logging de Erros** — Substituir `let _ =` por `if let Err(e) = ... log::warn!(...)`

---

## 📝 Notas Adicionais

### Pontos Positivos Encontrados:
- ✅ **Apenas 2 `unwrap()`** em 9.4k linhas (excelente disciplina)
- ✅ **Panic hook** grava logs para debug post-mortem
- ✅ **x11rb** (X11 protocol puro, sem Xlib overhead)
- ✅ **Sem global state** (dependency injection explícito)
- ✅ **Damage-pass rendering** (só redesenha regiões changed)
- ✅ **HashMap para O(1) window lookups**

### Features Bem Implementadas:
- ✅ `_NET_WM_STATE_FULLSCREEN` parsing
- ✅ RandR resize handling (reconfigure toolbar/slit/tray)
- ✅ System tray com collapse (Windows-style)
- ✅ Display correto para apps (mitigação Wayland hijack)
- ✅ Move/resize clamp ao workarea (não cobre taskbar)

---

**Conclusão:** O Rustbox é uma base sólida com arquitetura limpa, mas precisa de correções críticas em **fechamento de janelas**, **input de diálogo**, **vazamento de recursos** e **compatibilidade ICCCM** para ser production-ready.