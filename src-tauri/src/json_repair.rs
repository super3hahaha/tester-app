//! 兜底修一下模型输出里常见的“非法但可救”JSON 问题：字符串里忘了转义的内部引号、
//! 字面控制字符（裸换行/Tab）。只在正常 `extract_json_*` 之后、`serde_json::from_str`
//! 之前跑一遍；本身合法的 JSON 原样透传，不会把好输出改坏。
//!
//! 引号是否算“收尾”的判断：往后跳过空白，看紧跟的是不是 `,` `}` `]` `:` 或字符串末尾——
//! 像才当作真正收尾，否则当成模型漏转义的内部引号，补 `\"` 继续当字符串内容处理。
//! 这是启发式的，遇到「他说"很好"，然后……」这种引号后紧跟逗号的人类文本仍可能误判，
//! 但比完全不处理、直接让 serde_json 报错好。
pub fn repair_json(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() + 16);
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if escape {
            out.push(b);
            escape = false;
            i += 1;
            continue;
        }
        if in_string {
            match b {
                b'\\' => {
                    escape = true;
                    out.push(b);
                }
                b'"' => {
                    let mut j = i + 1;
                    while j < bytes.len() && matches!(bytes[j], b' ' | b'\t' | b'\r' | b'\n') {
                        j += 1;
                    }
                    let closes = j >= bytes.len() || matches!(bytes[j], b',' | b'}' | b']' | b':');
                    if closes {
                        in_string = false;
                        out.push(b'"');
                    } else {
                        out.push(b'\\');
                        out.push(b'"');
                    }
                }
                b'\n' => out.extend_from_slice(b"\\n"),
                b'\r' => out.extend_from_slice(b"\\r"),
                b'\t' => out.extend_from_slice(b"\\t"),
                0x00..=0x1F => {} // 其余控制字符丢弃：JSON 字符串里本就不允许字面出现
                _ => out.push(b),
            }
        } else {
            if b == b'"' {
                in_string = true;
            }
            out.push(b);
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::repair_json;

    #[test]
    fn fixes_unescaped_inner_quote() {
        let bad = r#"{"a": "he said "hi" to me", "b": 1}"#;
        let fixed = repair_json(bad);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["a"], "he said \"hi\" to me");
        assert_eq!(v["b"], 1);
    }

    #[test]
    fn fixes_raw_newline_in_string() {
        let bad = "{\"a\": \"line1\nline2\", \"b\":1}";
        let fixed = repair_json(bad);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["a"], "line1\nline2");
        assert_eq!(v["b"], 1);
    }

    #[test]
    fn leaves_valid_json_untouched() {
        let good = r#"{"a": "hello \"world\"", "b": [1, 2, 3]}"#;
        let fixed = repair_json(good);
        let v: serde_json::Value = serde_json::from_str(&fixed).unwrap();
        assert_eq!(v["a"], "hello \"world\"");
    }
}
