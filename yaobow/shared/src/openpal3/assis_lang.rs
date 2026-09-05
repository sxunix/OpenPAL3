//! `AssisLang.lang` / `*.dft` 文本表 —— `FUN_00457410` 的读法。
//!
//! 文件长这样：
//!
//! ```text
//! Begin,sec1
//! ; 注释到行尾
//! Lang000 $景天：\n老板，你别以为我不识货……&
//! Lang001 $商人：\n嗯~~~~~~，好吧。你开个价。&
//! end
//! ```
//!
//! tokenizer（`FUN_00431a60`）用空格、Tab、逗号、换行分词，`;` 起注释到行尾，
//! `$…&` 是字符串体，整个在分词层跳过。读表（`FUN_00457410`）先找到 `Begin`
//! 后面的节名，然后循环：取一个词，等于 `end`（不分大小写）就停，否则再读一个
//! `$…&` 字符串放进第 i 槽——**槽号是出现顺序**，`Lang081` 里的数字不参与。
//! 字符串里的 `\n` 是两个字符，显示时才换行（[`unescape`]）。
//!
//! 逆向笔记：`switch/re/notes/appraise_minigame.md`。

/// 原版每槽 `strncpy` 0x100 字节；这里按字节数截断到不切开 UTF-8 字符为止。
const SLOT_BYTES: usize = 0x100;

/// 把 `sec` 节的字符串按出现顺序读出来。文件是 GBK，调用方先转成 `&str`。
pub fn parse_section(text: &str, sec: &str) -> Vec<String> {
    let mut tokens = Tokenizer::new(text);
    // Begin 之后的第一个词是节名；原版就是找这个词，前面的东西都跳过。
    loop {
        match tokens.next_word() {
            Some(w) if w.eq_ignore_ascii_case("Begin") => break,
            Some(_) => continue,
            None => return vec![],
        }
    }
    loop {
        match tokens.next_word() {
            Some(w) if w == sec => break,
            Some(_) => continue,
            None => return vec![],
        }
    }

    let mut out = vec![];
    while let Some(word) = tokens.next_word() {
        if word.eq_ignore_ascii_case("end") {
            break;
        }
        let Some(s) = tokens.next_string() else { break };
        out.push(truncate_bytes(s, SLOT_BYTES));
    }
    out
}

/// 原版文本里的 `\n` 两个字符在显示时换行。
pub fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

struct Tokenizer<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.text[self.pos..]
    }

    fn is_sep(c: char) -> bool {
        matches!(c, ' ' | '\t' | ',' | '\r' | '\n')
    }

    /// 跳过分隔符和注释。
    fn skip_blank(&mut self) {
        loop {
            let rest = self.rest();
            let Some(c) = rest.chars().next() else { return };
            if Self::is_sep(c) {
                self.pos += c.len_utf8();
            } else if c == ';' {
                match rest.find('\n') {
                    Some(n) => self.pos += n + 1,
                    None => self.pos = self.text.len(),
                }
            } else {
                return;
            }
        }
    }

    /// 下一个词。碰到 `$…&` 字符串体整个跳过（原版在分词层就这么做）。
    fn next_word(&mut self) -> Option<&'a str> {
        loop {
            self.skip_blank();
            let rest = self.rest();
            let c = rest.chars().next()?;
            if c == '$' {
                self.skip_string();
                continue;
            }
            let end = rest.find(|c: char| Self::is_sep(c) || c == ';' || c == '$');
            let end = end.unwrap_or(rest.len());
            self.pos += end;
            return Some(&rest[..end]);
        }
    }

    fn skip_string(&mut self) {
        let rest = self.rest();
        debug_assert!(rest.starts_with('$'));
        match rest[1..].find('&') {
            Some(n) => self.pos += n + 2,
            None => self.pos = self.text.len(),
        }
    }

    /// 下一个 `$…&` 字符串（`FUN_00431b60`）。中间的词跳过。
    fn next_string(&mut self) -> Option<&'a str> {
        loop {
            self.skip_blank();
            let rest = self.rest();
            let c = rest.chars().next()?;
            if c == '$' {
                let body = &rest[1..];
                let n = body.find('&').unwrap_or(body.len());
                self.pos += 1 + n + 1;
                self.pos = self.pos.min(self.text.len());
                return Some(&body[..n]);
            }
            // 不是字符串就当普通词吃掉
            self.next_word()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Begin,sec1\r\n; 此文件最多语句数目不多于512条\r\nLang000 $景天：\\n老板&\r\nLang001 $商人：\\n嗯~~~~~~，好吧。&\r\nLang002 $名称&\r\nend\r\nLang003 $不该读到&\r\n";

    #[test]
    fn reads_in_order_and_stops_at_end() {
        let v = parse_section(SAMPLE, "sec1");
        assert_eq!(
            v,
            vec!["景天：\\n老板", "商人：\\n嗯~~~~~~，好吧。", "名称"]
        );
        assert_eq!(unescape(&v[0]), "景天：\n老板");
    }

    #[test]
    fn comments_and_commas_in_strings() {
        // 字符串体里的逗号、分号都不分词；注释行整行跳过。
        let text = "Begin sec1\n;Diffent0 $注释里的&\nDiffent0 $a,b;c&\nDiffent1 $x&\nEND";
        assert_eq!(parse_section(text, "sec1"), vec!["a,b;c", "x"]);
    }

    #[test]
    fn missing_section_is_empty() {
        assert!(parse_section(SAMPLE, "sec2").is_empty());
        assert!(parse_section("nothing here", "sec1").is_empty());
    }

    #[test]
    fn slots_are_capped_like_strncpy() {
        let long = "字".repeat(200); // 600 字节
        let text = format!("Begin,sec1\nX ${long}&\nend");
        let v = parse_section(&text, "sec1");
        assert_eq!(v[0].len(), 255); // 0x100 向下取到字符边界
        assert_eq!(v[0].chars().count(), 85);
    }
}
