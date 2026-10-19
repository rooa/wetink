//! Code block highlighting. With the `syntax` feature, fenced blocks with a
//! known language are highlighted by syntect; results are cached by content so
//! re-rendering an edited document only highlights code that changed.

use crate::render::Spans;
use crate::theme::t;
use ratatui::text::Span;
use std::collections::HashMap;

#[derive(Default)]
#[cfg_attr(not(feature = "syntax"), allow(dead_code))]
pub struct Highlighter {
    /// Results from the previous parse, and those reused or computed in this one.
    cache: HashMap<u64, Vec<Spans>>,
    used: HashMap<u64, Vec<Spans>>,
}

impl Highlighter {
    pub fn highlight(&mut self, lang: &str, code: &str) -> Vec<Spans> {
        #[cfg(feature = "syntax")]
        if !lang.is_empty() {
            use std::hash::{DefaultHasher, Hash, Hasher};
            let mut h = DefaultHasher::new();
            (lang, code).hash(&mut h);
            let key = h.finish();
            if let Some(v) = self.used.get(&key) {
                return v.clone();
            }
            if let Some(v) = self
                .cache
                .remove(&key)
                .or_else(|| syntax::highlight(lang, code))
            {
                self.used.insert(key, v.clone());
                return v;
            }
        }
        #[cfg(not(feature = "syntax"))]
        let _ = lang;
        code.split('\n')
            .map(|l| vec![Span::styled(l.to_string(), t().dim)])
            .collect()
    }

    /// Drop cache entries the last parse did not use.
    pub fn sweep(&mut self) {
        self.cache = std::mem::take(&mut self.used);
    }
}

#[cfg(feature = "syntax")]
mod syntax {
    use crate::render::Spans;
    use ratatui::style::{Color, Style};
    use ratatui::text::Span;
    use std::sync::OnceLock;
    use syntect::easy::HighlightLines;
    use syntect::highlighting::{Theme, ThemeSet};
    use syntect::parsing::SyntaxSet;
    use syntect::util::LinesWithEndings;

    fn assets() -> &'static (SyntaxSet, Theme) {
        static ASSETS: OnceLock<(SyntaxSet, Theme)> = OnceLock::new();
        ASSETS.get_or_init(|| {
            let mut themes = ThemeSet::load_defaults().themes;
            let theme = themes.remove(crate::theme::t().syntax).unwrap_or_default();
            (SyntaxSet::load_defaults_newlines(), theme)
        })
    }

    pub fn highlight(lang: &str, code: &str) -> Option<Vec<Spans>> {
        let (ss, theme) = assets();
        // syntect bundles no TypeScript grammar; JavaScript is close enough.
        let lang = match lang {
            "ts" | "typescript" | "tsx" | "mts" | "cts" => "js",
            l => l,
        };
        let syntax = ss.find_syntax_by_token(lang)?;
        let mut h = HighlightLines::new(syntax, theme);
        let mut out = Vec::new();
        for line in LinesWithEndings::from(code) {
            let spans = h
                .highlight_line(line, ss)
                .ok()?
                .into_iter()
                .filter_map(|(st, t)| {
                    let t = t.trim_end_matches(['\n', '\r']);
                    let fg = Color::Rgb(st.foreground.r, st.foreground.g, st.foreground.b);
                    (!t.is_empty()).then(|| Span::styled(t.to_string(), Style::new().fg(fg)))
                })
                .collect();
            out.push(spans);
        }
        Some(out)
    }
}
