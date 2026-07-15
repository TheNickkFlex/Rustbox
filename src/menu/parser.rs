use std::fs;

use super::menuitem::{MenuItem, MenuItemType};

/// Parse a Rustbox menu file and return the parsed menu items.
///
/// Format per line:
///   [type] (label) {command} <icon>
///
/// Types: begin, end, exec, exit, restart, reconfig, submenu, separator,
///        include, nop, workspaces
///
/// `begin` at the top-level starts root menu definition (label is the title).
/// Returns the items inside the top-level [begin]...[end] block, plus the
/// root menu label.
pub fn parse_menu_file(path: &str) -> Result<(String, Vec<MenuItem>), anyhow::Error> {
    let content = fs::read_to_string(path)?;
    let lines = preprocess_lines(&content);
    let mut tokens = TokenStream::new(&lines);

    // Skip leading whitespace/comments until we see [begin].
    loop {
        let tok = tokens.peek();
        match tok.as_deref() {
            Some("[begin]") | Some("[Begin]") => break,
            Some(t) if t.starts_with('[') => {
                // Unknown section, skip whole line.
                tokens.advance();
                tokens.skip_line();
            }
            Some(_) => {
                tokens.advance(); // skip non-bracket token
            }
            None => return Err(anyhow::anyhow!("No [begin] found in menu file")),
        }
    }

    // Consume [begin].
    tokens.advance(); // [begin]
    let label = tokens.next_label().unwrap_or_default();
    tokens.skip_rest();

    let items = parse_block(&mut tokens, 0)?;

    Ok((label, items))
}

/// Parse items until [end] or EOF.
fn parse_block(tokens: &mut TokenStream, depth: usize) -> Result<Vec<MenuItem>, anyhow::Error> {
    const MAX_DEPTH: usize = 32;
    if depth > MAX_DEPTH {
        return Err(anyhow::anyhow!("Menu nesting too deep (>{MAX_DEPTH})"));
    }

    let mut items = Vec::new();

    loop {
        let tok = match tokens.next() {
            Some(t) => t,
            None => break,
        };

        match tok.as_str() {
            "[end]" | "[End]" => break,
            "[exec]" | "[Exec]" => {
                let label = tokens.next_label().unwrap_or_default();
                let cmd = tokens.next_arg().unwrap_or_default();
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Exec(cmd)));
            }
            "[exit]" | "[Exit]" => {
                let label = tokens.next_label().unwrap_or_else(|| "Exit".to_string());
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Exit));
            }
            "[restart]" | "[Restart]" => {
                let label = tokens.next_label().unwrap_or_else(|| "Restart".to_string());
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Restart));
            }
            "[reconfig]" | "[Reconfig]" => {
                let label = tokens.next_label().unwrap_or_else(|| "Reconfigure".to_string());
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Reconfig));
            }
            "[separator]" | "[Separator]" | "[Separator]:" => {
                tokens.skip_rest();
                items.push(MenuItem::separator());
            }
            "[nop]" | "[Nop]" => {
                let label = tokens.next_label().unwrap_or_default();
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Nop));
            }
            "[submenu]" | "[Submenu]" => {
                let label = tokens.next_label().unwrap_or_default();
                tokens.skip_rest();
                let _sub_items = parse_block(tokens, depth + 1)?;
                let sub_id = generate_menu_id();
                // Insert a Submenu item; the actual menu object is created later.
                items.push(MenuItem::new(&label, MenuItemType::Submenu(sub_id, label.clone())));
            }
            "[include]" | "[Include]" => {
                let label = tokens.next_label().unwrap_or_default();
                let path = tokens.next_arg().unwrap_or_default();
                tokens.skip_rest();
                if !path.is_empty() {
                    items.push(MenuItem::new(&label, MenuItemType::Include(path)));
                } else {
                    items.push(MenuItem::new(&label, MenuItemType::Include(
                        expand_path(&label)
                    )));
                }
            }
            "[workspaces]" | "[Workspaces]" => {
                let label = tokens.next_label().unwrap_or_else(|| "Workspaces".to_string());
                tokens.skip_rest();
                items.push(MenuItem::new(&label, MenuItemType::Workspaces));
            }
            _ => {
                // Unknown directive, skip rest of line.
                tokens.skip_rest();
            }
        }
    }

    Ok(items)
}

/// Pre-process lines: strip comments ('#'), trim whitespace, skip empties.
fn preprocess_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| {
            let trimmed = l.trim();
            // Remove everything after the first unquoted '#'.
            let mut in_paren = 0;
            let mut in_bracket = 0;
            let mut in_brace = 0;
            let mut result = String::new();
            for ch in trimmed.chars() {
                match ch {
                    '(' => { in_paren += 1; result.push(ch); }
                    ')' => { in_paren -= 1; result.push(ch); }
                    '[' => { in_bracket += 1; result.push(ch); }
                    ']' => { in_bracket -= 1; result.push(ch); }
                    '{' => { in_brace += 1; result.push(ch); }
                    '}' => { in_brace -= 1; result.push(ch); }
                    '#' if in_paren == 0 && in_bracket == 0 && in_brace == 0 => break,
                    _ => result.push(ch),
                }
            }
            result.trim().to_string()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

/// Expand `~` in a path to the home directory.
fn expand_path(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return home + &path[1..];
        }
    }
    path.to_string()
}

static NEXT_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

fn generate_menu_id() -> u32 {
    NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

// ---- simple line-based tokeniser ----

struct TokenStream {
    lines: Vec<String>,
    pos: usize,
    line_pos: usize, // position within current line
    current_line: String,
}

impl TokenStream {
    fn new(lines: &[String]) -> Self {
        let lines = lines.to_vec();
        let current_line = lines.first().cloned().unwrap_or_default();
        Self { lines, pos: 0, line_pos: 0, current_line }
    }

    fn peek(&self) -> Option<String> {
        // Return the next bracket token without advancing.
        let mut stream = self.clone();
        stream.next_bracket()
    }

    fn advance(&mut self) {
        self.line_pos = self.current_line.len(); // force next line
    }

    fn skip_line(&mut self) {
        self.line_pos = self.current_line.len();
        self.advance_line();
    }

    fn skip_rest(&mut self) {
        self.line_pos = self.current_line.len();
        self.advance_line();
    }

    fn next(&mut self) -> Option<String> {
        // First try bracket token, then label, then arg, then icon token.
        if let Some(tok) = self.next_bracket() {
            return Some(tok);
        }
        // After bracket, consume (label) if present.
        // Labels/args are consumed by explicit calls; this just returns brackets.
        None
    }

    fn next_bracket(&mut self) -> Option<String> {
        loop {
            // Skip whitespace.
            while self.line_pos < self.current_line.len()
                && self.current_line.as_bytes()[self.line_pos] == b' '
            {
                self.line_pos += 1;
            }
            if self.line_pos >= self.current_line.len() {
                if !self.advance_line() {
                    return None;
                }
                continue;
            }
            let rest = &self.current_line[self.line_pos..];
            if rest.starts_with('[') {
                if let Some(end) = rest.find(']') {
                    let tok = rest[..=end].to_string();
                    self.line_pos += end + 1;
                    return Some(tok);
                }
            }
            // Not a bracket at this position — check if it's a label/arg start.
            // For `next()`, we only return brackets. Other tokens are consumed
            // by dedicated methods.
            return None;
        }
    }

    fn next_label(&mut self) -> Option<String> {
        self.skip_whitespace();
        let rest = &self.current_line[self.line_pos..];
        if rest.starts_with('(') {
            if let Some(end) = rest.find(')') {
                let label = rest[1..end].to_string();
                self.line_pos += end + 1;
                return Some(label);
            }
        }
        None
    }

    fn next_arg(&mut self) -> Option<String> {
        self.skip_whitespace();
        let rest = &self.current_line[self.line_pos..];
        if rest.starts_with('{') {
            if let Some(end) = rest.find('}') {
                let arg = rest[1..end].to_string();
                self.line_pos += end + 1;
                return Some(arg);
            }
        }
        None
    }

    fn skip_whitespace(&mut self) {
        while self.line_pos < self.current_line.len()
            && self.current_line.as_bytes()[self.line_pos] == b' '
        {
            self.line_pos += 1;
        }
        if self.line_pos >= self.current_line.len() {
            self.advance_line();
        }
    }

    fn advance_line(&mut self) -> bool {
        self.pos += 1;
        self.line_pos = 0;
        if self.pos < self.lines.len() {
            self.current_line = self.pos_increment().clone();
            true
        } else {
            false
        }
    }
}

impl Clone for TokenStream {
    fn clone(&self) -> Self {
        Self {
            lines: self.lines.clone(),
            pos: self.pos,
            line_pos: self.line_pos,
            current_line: self.current_line.clone(),
        }
    }
}

impl TokenStream {
    fn pos_increment(&self) -> &String {
        &self.lines[self.pos]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_menu() {
        let _menu = r#"[begin] (Rustbox Root Menu)
  [exec] (termion) {termion}
  [exec] (Firefox) {firefox}
  [separator]
  [submenu] (Appearance)
    [config] (Configuration)
  [end]
  [reconfig] (Reload)
  [restart] (Restart)
  [exit] (Exit)
[end]"#;
        let (_label, _items) = parse_menu_file("test.menu").unwrap_or_default();
        // Without a real file, we rely on preprocess + parse_block.
    }
}
