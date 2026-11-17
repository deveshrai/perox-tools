//! Completion: the item model, fuzzy matching and ranking, and the popup's state.
//! Items come from rust-analyzer; as the user keeps typing they are re-filtered here
//! on every keystroke, so the list updates instantly, without waiting for the server.

use std::collections::HashMap;

use crate::text::Change;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Function,
    Method,
    Field,
    Variable,
    Constant,
    Struct,
    Enum,
    EnumMember,
    Trait,
    Module,
    Keyword,
    Snippet,
    TypeParam,
    Macro,
    Other,
}

impl Kind {
    /// A short tag shown in the popup.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Function => "fn",
            Kind::Method => "me",
            Kind::Field => "fd",
            Kind::Variable => "va",
            Kind::Constant => "ct",
            Kind::Struct => "st",
            Kind::Enum => "en",
            Kind::EnumMember => "ev",
            Kind::Trait => "tr",
            Kind::Module => "md",
            Kind::Keyword => "kw",
            Kind::Snippet => "sn",
            Kind::TypeParam => "tp",
            Kind::Macro => "ma",
            Kind::Other => "··",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub label: String,
    /// Shown dimmed after the label, e.g. `(…)` or ` (use std::collections::HashMap)`.
    pub label_detail: Option<String>,
    /// Usually the type or signature.
    pub detail: Option<String>,
    pub kind: Kind,
    pub filter_text: String,
    pub sort_text: String,
    pub insert_text: String,
    pub is_snippet: bool,
    /// Chars before the cursor that the item replaces, at request time.
    pub prefix_len: usize,
    /// Chars after the cursor that the item replaces.
    pub suffix_len: usize,
    /// E.g. the `use` line for auto-imports.
    pub additional_edits: Vec<Change>,
    pub documentation: Option<String>,
    pub deprecated: bool,
    pub preselect: bool,
    /// Index in the server's list, to resolve details lazily.
    pub server_index: usize,
    pub resolved: bool,
}

/// Fuzzy match score of `pattern` against `candidate`, or `None` if the pattern's
/// characters don't all appear in order. Higher is better. Rewards a prefix match,
/// matches at word starts (`_`, `::`, camelCase), consecutive runs, and exact case.
pub fn fuzzy_score(pattern: &str, candidate: &str) -> Option<i32> {
    if pattern.is_empty() {
        return Some(0);
    }
    let p: Vec<char> = pattern.chars().collect();
    let c: Vec<char> = candidate.chars().collect();
    if p.len() > c.len() {
        return None;
    }
    let lower = |ch: char| ch.to_lowercase().next().unwrap_or(ch);
    let boundary = |i: usize| {
        i == 0 || {
            let prev = c[i - 1];
            !prev.is_alphanumeric() || (prev.is_lowercase() && c[i].is_uppercase())
        }
    };
    // dp[j] = best score with p[..=i] matched and p[i] at c[j]; NEG if impossible.
    const NEG: i32 = i32::MIN / 4;
    let mut prev_row = vec![NEG; c.len()];
    for (i, &pc) in p.iter().enumerate() {
        let mut row = vec![NEG; c.len()];
        let mut best_before = NEG; // max over prev_row[..j] minus gap penalties
        for j in 0..c.len() {
            if i > 0 && j > 0 {
                best_before = (best_before - 1).max(prev_row[j - 1]);
            }
            if lower(c[j]) != lower(pc) {
                continue;
            }
            let mut s = 1;
            if c[j] == pc {
                s += 2; // matching case shows intent: `Str` means `String`, not `str`
            }
            if boundary(j) {
                s += 8;
            }
            if i == 0 {
                // Penalise starting late, a little.
                row[j] = s - (j as i32).min(10);
                if j == 0 {
                    row[j] += 12;
                }
            } else {
                let consecutive = if j > 0 && prev_row[j - 1] > NEG {
                    prev_row[j - 1] + s + 6
                } else {
                    NEG
                };
                let gapped = if best_before > NEG {
                    best_before + s
                } else {
                    NEG
                };
                row[j] = consecutive.max(gapped);
            }
        }
        prev_row = row;
    }
    let best = prev_row.into_iter().max().filter(|s| *s > NEG)?;
    // Exact prefix (case-insensitive) is what people expect first; among prefixes, the
    // closest (fewest extra characters) wins: `fr` → `from` before `from_raw_parts`.
    let prefix = candidate
        .to_lowercase()
        .starts_with(&pattern.to_lowercase());
    let extra = (c.len() - p.len()).min(12) as i32;
    Some(best + if prefix { 20 - extra / 3 } else { 0 })
}

/// Recently accepted completions float up.
#[derive(Debug, Default, Clone)]
pub struct Recency {
    tick: u64,
    used: HashMap<String, u64>,
}

impl Recency {
    pub fn record(&mut self, label: &str) {
        self.tick += 1;
        self.used.insert(label.to_owned(), self.tick);
        if self.used.len() > 500 {
            let cutoff = self.tick.saturating_sub(400);
            self.used.retain(|_, t| *t > cutoff);
        }
    }

    fn bonus(&self, label: &str) -> i32 {
        match self.used.get(label) {
            Some(t) => {
                let age = self.tick - t;
                (12 - (age as i32).min(12)).max(3)
            }
            None => 0,
        }
    }
}

/// The open completion popup.
#[derive(Debug, Clone)]
pub struct Popup {
    pub items: Vec<Item>,
    /// Indices into `items`, best first.
    pub shown: Vec<usize>,
    pub selected: usize,
    /// Char index where the word being completed starts.
    pub word_start: usize,
    /// The server said the list is incomplete: ask again as the user types.
    pub incomplete: bool,
}

impl Popup {
    pub fn new(items: Vec<Item>, word_start: usize, incomplete: bool) -> Self {
        Self {
            items,
            shown: Vec::new(),
            selected: 0,
            word_start,
            incomplete,
        }
    }

    /// Re-filters for the word typed so far. Keeps the selection on the same item if
    /// it still matches; otherwise selects the best (or a preselected one).
    pub fn filter(&mut self, typed: &str, recency: &Recency) {
        let previously = self.shown.get(self.selected).copied();
        let mut scored: Vec<(i32, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let score = fuzzy_score(typed, &it.filter_text)?;
                let mut s = score + recency.bonus(&it.label);
                if it.deprecated {
                    s -= 15;
                }
                // Names already in scope beat ones that need an import; platform intrinsics
                // (`core::arch`) are almost never what's meant.
                if let Some(detail) = &it.label_detail
                    && detail.contains("(use ")
                {
                    s -= if detail.contains("::arch::") { 40 } else { 6 };
                }
                Some((s, i))
            })
            .collect();
        // Score first; the server's relevance order (sort_text) breaks ties. With nothing
        // typed, the server's order alone.
        scored.sort_by(|a, b| {
            let (ia, ib) = (&self.items[a.1], &self.items[b.1]);
            if typed.is_empty() {
                ia.sort_text.cmp(&ib.sort_text)
            } else {
                b.0.cmp(&a.0).then_with(|| ia.sort_text.cmp(&ib.sort_text))
            }
            .then_with(|| ia.label.len().cmp(&ib.label.len()))
        });
        self.shown = scored.into_iter().map(|(_, i)| i).collect();
        self.selected = previously
            .and_then(|p| self.shown.iter().position(|i| *i == p))
            .filter(|_| typed.is_empty())
            .or_else(|| {
                typed
                    .is_empty()
                    .then(|| self.shown.iter().position(|i| self.items[*i].preselect))
                    .flatten()
            })
            .unwrap_or(0);
    }

    pub fn current(&self) -> Option<&Item> {
        self.shown.get(self.selected).map(|i| &self.items[*i])
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.shown.is_empty() {
            return;
        }
        let n = self.shown.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(n)) as usize;
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, sort: &str) -> Item {
        Item {
            label: label.into(),
            label_detail: None,
            detail: None,
            kind: Kind::Function,
            filter_text: label.into(),
            sort_text: sort.into(),
            insert_text: label.into(),
            is_snippet: false,
            prefix_len: 0,
            suffix_len: 0,
            additional_edits: vec![],
            documentation: None,
            deprecated: false,
            preselect: false,
            server_index: 0,
            resolved: false,
        }
    }

    fn order(labels: &[&str], typed: &str) -> Vec<String> {
        let items = labels
            .iter()
            .enumerate()
            .map(|(i, l)| item(l, &format!("{i:04}")))
            .collect();
        let mut p = Popup::new(items, 0, false);
        p.filter(typed, &Recency::default());
        p.shown.iter().map(|i| p.items[*i].label.clone()).collect()
    }

    #[test]
    fn no_match_without_subsequence() {
        assert!(fuzzy_score("xyz", "with_capacity").is_none());
        assert!(fuzzy_score("wc", "with_capacity").is_some());
    }

    #[test]
    fn prefix_beats_word_boundaries_beats_scattered() {
        let a = fuzzy_score("wi", "with_capacity").unwrap();
        let b = fuzzy_score("wi", "new_with").unwrap();
        let c = fuzzy_score("wi", "sweeping").unwrap();
        assert!(a > b && b > c, "{a} {b} {c}");
    }

    #[test]
    fn word_start_abbreviations_rank_well() {
        assert_eq!(
            order(&["shrink_to_fit", "with_capacity", "swap_with_slice"], "wc"),
            ["with_capacity", "swap_with_slice"]
        );
        assert_eq!(order(&["HashSet", "HashMap", "hash"], "HM")[0], "HashMap");
    }

    #[test]
    fn nothing_typed_keeps_server_order() {
        assert_eq!(order(&["b", "a", "c"], ""), ["b", "a", "c"]);
    }

    #[test]
    fn recency_promotes_used_items() {
        let items = vec![item("len", "0"), item("last", "1")];
        let mut p = Popup::new(items, 0, false);
        let mut r = Recency::default();
        p.filter("l", &r);
        assert_eq!(p.current().unwrap().label, "len");
        r.record("last");
        p.filter("l", &r);
        assert_eq!(p.current().unwrap().label, "last");
    }

    #[test]
    fn in_scope_beats_auto_import_and_intrinsics_sink() {
        let mut vec_macro = item("vec![…]", "ffffff00");
        vec_macro.filter_text = "vec!".into();
        let mut intrinsic = item("veor_s8(…)", "fffffff0");
        intrinsic.filter_text = "veor_s8".into();
        intrinsic.label_detail = Some(" (use core::arch::aarch64::veor_s8)".into());
        let mut p = Popup::new(vec![intrinsic, vec_macro], 0, false);
        p.filter("ve", &Recency::default());
        assert_eq!(p.current().unwrap().label, "vec![…]");
    }

    #[test]
    fn closest_prefix_wins() {
        assert_eq!(
            order(&["from_raw_parts", "from_utf16_lossy", "from"], "fr")[0],
            "from"
        );
        assert_eq!(order(&["iter_mut", "iter"], "it")[0], "iter");
    }

    #[test]
    fn exact_case_wins_ties() {
        assert!(fuzzy_score("Str", "String").unwrap() > fuzzy_score("Str", "str").unwrap());
    }
}
