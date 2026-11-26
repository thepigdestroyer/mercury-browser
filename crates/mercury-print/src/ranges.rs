// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! Which pages of a document a print job or a screenshot covers.

use std::fmt;

/// The 1-based pages to print, sorted and without duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSelection {
    pages: Vec<usize>,
}

/// Why a page range text was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// The document has no pages.
    Empty,
    /// The text is not a list of pages and ranges; the payload is the offending item.
    Invalid(String),
    /// A page past the last page of the document.
    OutOfRange { page: usize, total: usize },
    /// A range whose end comes before its start.
    Reversed(usize, usize),
}

impl PageSelection {
    /// Parses a page range such as `1-3, 5, 8-`. Empty text and `all` mean every page.
    pub fn parse(text: &str, total_pages: usize) -> Result<PageSelection, RangeError> {
        if total_pages == 0 {
            return Err(RangeError::Empty);
        }
        let text = text.trim();
        if text.is_empty() || text.eq_ignore_ascii_case("all") {
            return Ok(Self::all(total_pages));
        }

        let normalized: String = text
            .chars()
            .map(|c| {
                if matches!(c, '–' | '—' | '‒' | '−') {
                    '-'
                } else {
                    c
                }
            })
            .collect();

        let mut ranges = Vec::new();
        for item in normalized.split([',', ';']) {
            let item = item.trim();
            if !item.is_empty() {
                ranges.push(parse_item(item, total_pages)?);
            }
        }
        if ranges.is_empty() {
            return Err(RangeError::Invalid(text.to_owned()));
        }

        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in ranges {
            match merged.last_mut() {
                Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        let pages = merged
            .into_iter()
            .flat_map(|(start, end)| start..=end)
            .collect();
        Ok(PageSelection { pages })
    }

    /// Every page of a document with `total_pages` pages.
    pub fn all(total_pages: usize) -> PageSelection {
        PageSelection {
            pages: (1..=total_pages).collect(),
        }
    }

    /// The selected pages, 1-based and ascending.
    pub fn pages(&self) -> &[usize] {
        &self.pages
    }

    /// The selection as range text, for example `1-3, 5, 8-10`.
    pub fn to_text(&self) -> String {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for &page in &self.pages {
            match runs.last_mut() {
                Some(run) if run.1 + 1 == page => run.1 = page,
                _ => runs.push((page, page)),
            }
        }
        runs.iter()
            .map(|&(start, end)| {
                if start == end {
                    start.to_string()
                } else {
                    format!("{start}-{end}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Whether no page is selected.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }
}

/// The number of copies to print: at least 1 and at most 99.
pub fn clamp_copies(copies: u32) -> u32 {
    copies.clamp(1, 99)
}

fn parse_item(item: &str, total: usize) -> Result<(usize, usize), RangeError> {
    let invalid = || RangeError::Invalid(item.to_owned());
    let (start, end, open_end) = match item.split_once('-') {
        None => {
            let page = parse_page(item).ok_or_else(invalid)?;
            (page, page, false)
        }
        Some((from, to)) => {
            let (from, to) = (from.trim(), to.trim());
            if from.is_empty() && to.is_empty() {
                return Err(invalid());
            }
            let start = if from.is_empty() {
                1
            } else {
                parse_page(from).ok_or_else(invalid)?
            };
            let open_end = to.is_empty();
            let end = if open_end {
                total
            } else {
                parse_page(to).ok_or_else(invalid)?
            };
            (start, end, open_end)
        }
    };

    if !open_end && start > end {
        return Err(RangeError::Reversed(start, end));
    }
    if start > total {
        return Err(RangeError::OutOfRange { page: start, total });
    }
    if end > total {
        return Err(RangeError::OutOfRange { page: end, total });
    }
    Ok((start, end))
}

fn parse_page(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse::<usize>().ok().filter(|&page| page >= 1)
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeError::Empty => write!(f, "the document has no pages"),
            RangeError::Invalid(item) => write!(f, "“{item}” is not a page range"),
            RangeError::OutOfRange { page, total } => {
                write!(f, "page {page} is past the last page ({total})")
            }
            RangeError::Reversed(start, end) => write!(f, "the range {start}-{end} runs backwards"),
        }
    }
}

impl std::error::Error for RangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(text: &str, total: usize) -> Vec<usize> {
        PageSelection::parse(text, total)
            .expect("valid range")
            .pages()
            .to_vec()
    }

    #[test]
    fn empty_and_all_mean_every_page() {
        assert_eq!(pages("", 4), vec![1, 2, 3, 4]);
        assert_eq!(pages(" ALL ", 4), vec![1, 2, 3, 4]);
    }

    #[test]
    fn single_range_list() {
        assert_eq!(pages("1-3, 5, 8-", 10), vec![1, 2, 3, 5, 8, 9, 10]);
    }

    #[test]
    fn open_start_range() {
        assert_eq!(pages("-3", 10), vec![1, 2, 3]);
    }

    #[test]
    fn duplicates_and_overlaps_collapse() {
        assert_eq!(pages("1-3,2-4,2", 10), vec![1, 2, 3, 4]);
    }

    #[test]
    fn semicolons_and_spaces() {
        assert_eq!(pages("1;3", 4), vec![1, 3]);
        assert_eq!(pages(" 2 ,  4 ", 4), vec![2, 4]);
        assert_eq!(
            PageSelection::parse("1 2", 4),
            Err(RangeError::Invalid("1 2".to_owned()))
        );
    }

    #[test]
    fn reversed_is_error() {
        assert_eq!(
            PageSelection::parse("3-1", 10),
            Err(RangeError::Reversed(3, 1))
        );
    }

    #[test]
    fn zero_is_invalid() {
        assert!(matches!(
            PageSelection::parse("0", 4),
            Err(RangeError::Invalid(_))
        ));
        assert!(matches!(
            PageSelection::parse("0-2", 4),
            Err(RangeError::Invalid(_))
        ));
    }

    #[test]
    fn out_of_range() {
        assert_eq!(
            PageSelection::parse("5", 4),
            Err(RangeError::OutOfRange { page: 5, total: 4 })
        );
        assert_eq!(
            PageSelection::parse("3-6", 4),
            Err(RangeError::OutOfRange { page: 6, total: 4 })
        );
    }

    #[test]
    fn unicode_dashes() {
        assert_eq!(pages("1–3", 5), vec![1, 2, 3]);
        assert_eq!(pages("2—4", 5), vec![2, 3, 4]);
    }

    #[test]
    fn empty_items_are_ignored() {
        assert_eq!(pages("1,,3", 4), vec![1, 3]);
    }

    #[test]
    fn huge_input_is_bounded() {
        let digits = "9".repeat(10_000);
        assert!(matches!(
            PageSelection::parse(&digits, 4),
            Err(RangeError::Invalid(_))
        ));

        let many = vec!["1"; 5_000].join(",");
        assert_eq!(pages(&many, 4), vec![1]);
    }

    #[test]
    fn to_text_round_trips() {
        let selection = PageSelection::parse("1-3, 5, 8-", 10).expect("valid range");
        assert_eq!(selection.to_text(), "1-3, 5, 8-10");
        assert_eq!(
            PageSelection::parse(&selection.to_text(), 10).expect("valid range"),
            selection
        );
    }

    #[test]
    fn zero_total_is_empty_error() {
        assert_eq!(PageSelection::parse("all", 0), Err(RangeError::Empty));
    }

    #[test]
    fn clamp_copies_keeps_between_one_and_99() {
        assert_eq!(clamp_copies(0), 1);
        assert_eq!(clamp_copies(5), 5);
        assert_eq!(clamp_copies(1000), 99);
    }
}
