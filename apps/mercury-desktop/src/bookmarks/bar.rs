//! The bookmarks bar: the children of the bar root as buttons, laid out by
//! [`layout::bar_layout`] from the widths the buttons report, with the rest behind "»", and a
//! popup for folders. A folder inside a popup opens in the same popup, with a back row at the
//! top (one popup, not a cascade of submenus).

use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use mercury_store::{BookmarkId, BookmarkKind, BookmarkNode, BookmarkRoot};
use mercury_ui::{BookmarkBarItem, BookmarkMenuRow, BookmarkRowKind, MainWindow};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::layout::{BarLayout, bar_layout};
use super::{Bookmarks, with};
use crate::profile_data;

/// The most buttons the bar creates; later entries are only reachable through "»". More than
/// fit across any screen.
pub(crate) const MAX_BAR_BUTTONS: usize = 200;

/// What a popup shows: the levels opened, outermost first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MenuLevel {
    Overflow,
    Folder { id: BookmarkId, title: String },
}

/// One row of a popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MenuEntry {
    Back { title: String },
    Node(BookmarkNode),
    Empty,
}

/// A popup's rows for the innermost of `levels`: a back row naming the level before it when
/// there is one, then `nodes`, or "Empty".
pub(crate) fn menu_entries(levels: &[MenuLevel], nodes: Vec<BookmarkNode>) -> Vec<MenuEntry> {
    let mut entries = Vec::with_capacity(nodes.len() + 1);
    if let [.., outer, _] = levels {
        entries.push(MenuEntry::Back {
            title: match outer {
                MenuLevel::Overflow => "More bookmarks".to_owned(),
                MenuLevel::Folder { title, .. } => title.clone(),
            },
        });
    }
    if nodes.is_empty() {
        entries.push(MenuEntry::Empty);
    }
    entries.extend(nodes.into_iter().map(MenuEntry::Node));
    entries
}

pub(crate) fn menu_row(entry: &MenuEntry) -> BookmarkMenuRow {
    let (title, url, kind) = match entry {
        MenuEntry::Back { title } => (title.as_str(), "", BookmarkRowKind::Back),
        MenuEntry::Empty => ("Empty", "", BookmarkRowKind::Empty),
        MenuEntry::Node(node) => match &node.kind {
            BookmarkKind::Bookmark { url } => {
                (node.title.as_str(), url.as_str(), BookmarkRowKind::Bookmark)
            }
            BookmarkKind::Folder => (node.title.as_str(), "", BookmarkRowKind::Folder),
            BookmarkKind::Separator => ("", "", BookmarkRowKind::Separator),
        },
    };
    BookmarkMenuRow {
        title: title.into(),
        url: url.into(),
        kind,
        depth: 0,
    }
}

/// The widths to lay out: one per button, and an unmeasured one standing for the entries
/// beyond [`MAX_BAR_BUTTONS`] so that they get the "»" button.
pub(crate) fn layout_widths(widths: &[Option<f32>], entries: usize) -> Vec<Option<f32>> {
    let mut widths = widths.to_vec();
    if entries > widths.len() {
        widths.push(None);
    }
    widths
}

/// Whether a change to the listings of `folders` (`None`: any) can change the bar, whose
/// folder is `root` (`None` until it has been read).
pub(crate) fn affected_by(root: Option<BookmarkId>, folders: Option<&[BookmarkId]>) -> bool {
    match (root, folders) {
        (Some(root), Some(folders)) => folders.contains(&root),
        _ => true,
    }
}

pub(crate) struct Bar {
    root: Option<BookmarkId>,
    nodes: Vec<BookmarkNode>,
    widths: Vec<Option<f32>>,
    model: Rc<VecModel<BookmarkBarItem>>,
    layout: BarLayout,
    generation: u64,
    relayout_pending: bool,
    levels: Vec<MenuLevel>,
    entries: Vec<MenuEntry>,
    menu_generation: u64,
}

impl Bar {
    pub(crate) fn new() -> Self {
        Self {
            root: None,
            nodes: Vec::new(),
            widths: Vec::new(),
            model: Rc::new(VecModel::default()),
            layout: bar_layout(&[], 0.0),
            generation: 0,
            relayout_pending: false,
            levels: Vec::new(),
            entries: Vec::new(),
            menu_generation: 0,
        }
    }

    /// The bar's folder, once read.
    pub(crate) fn root(&self) -> Option<BookmarkId> {
        self.root
    }

    /// The entries of the bar root, in order (the Bookmarks menu lists them).
    #[expect(
        dead_code,
        reason = "called by the menus lane's Bookmarks seam, not merged yet"
    )]
    pub(crate) fn nodes(&self) -> &[BookmarkNode] {
        &self.nodes
    }
}

pub(crate) fn connect(window: &MainWindow) {
    with(|bookmarks| {
        let model = Rc::clone(&bookmarks.bar.borrow().model);
        window.set_bookmarks_bar_items(ModelRc::from(model));
    });
    window.on_bookmarks_bar_measured(|index, width| {
        with(|bookmarks| {
            let mut bar = bookmarks.bar.borrow_mut();
            if let Some(slot) = usize::try_from(index)
                .ok()
                .and_then(|index| bar.widths.get_mut(index))
            {
                *slot = Some(width);
            }
            // Laid out after the buttons are made: not from inside the repeater's creation.
            if !bar.relayout_pending {
                bar.relayout_pending = true;
                let bookmarks = Rc::downgrade(bookmarks);
                slint::Timer::single_shot(Duration::ZERO, move || {
                    if let Some(bookmarks) = bookmarks.upgrade() {
                        relayout(&bookmarks);
                    }
                });
            }
        });
    });
    window.on_bookmarks_bar_width_changed(|_| {
        with(relayout);
    });
    window.on_bookmarks_bar_item_activated(|index, new_tab| {
        with(|bookmarks| {
            let url = usize::try_from(index)
                .ok()
                .and_then(|index| bookmarks.bar.borrow().nodes.get(index).cloned())
                .and_then(|node| match node.kind {
                    BookmarkKind::Bookmark { url } => Some(url),
                    _ => None,
                });
            if let Some(url) = url {
                bookmarks.open_url(&url, new_tab);
            }
        });
    });
    window.on_bookmarks_bar_folder_opened(|index, x| {
        with(|bookmarks| {
            let node = usize::try_from(index)
                .ok()
                .and_then(|index| bookmarks.bar.borrow().nodes.get(index).cloned());
            if let Some(node) = node {
                bookmarks.bar.borrow_mut().levels = vec![MenuLevel::Folder {
                    id: node.id,
                    title: node.title,
                }];
                load_menu(bookmarks, Some(x));
            }
        });
    });
    window.on_bookmarks_bar_overflow_opened(|x| {
        with(|bookmarks| {
            bookmarks.bar.borrow_mut().levels = vec![MenuLevel::Overflow];
            load_menu(bookmarks, Some(x));
        });
    });
    window.on_bookmarks_menu_row_activated(|index, new_tab| {
        with(|bookmarks| {
            let entry = usize::try_from(index)
                .ok()
                .and_then(|index| bookmarks.bar.borrow().entries.get(index).cloned());
            match entry {
                Some(MenuEntry::Back { .. }) => {
                    bookmarks.bar.borrow_mut().levels.pop();
                    load_menu(bookmarks, None);
                }
                Some(MenuEntry::Node(node)) => match node.kind {
                    BookmarkKind::Folder => {
                        bookmarks.bar.borrow_mut().levels.push(MenuLevel::Folder {
                            id: node.id,
                            title: node.title,
                        });
                        load_menu(bookmarks, None);
                    }
                    BookmarkKind::Bookmark { url } => {
                        if let Some(window) = bookmarks.window.upgrade() {
                            window.invoke_close_bookmarks_menu();
                        }
                        bookmarks.open_url(&url, new_tab);
                    }
                    BookmarkKind::Separator => {}
                },
                Some(MenuEntry::Empty) | None => {}
            }
        });
    });
}

/// Reads the bar root's children again and rebuilds the buttons. Widths already measured for
/// the same title are kept, so unchanged buttons do not disappear until they are measured.
pub(crate) fn reload(bookmarks: &Rc<Bookmarks>) {
    let Some(data) = profile_data::handle() else {
        return;
    };
    let generation = {
        let mut bar = bookmarks.bar.borrow_mut();
        bar.generation += 1;
        bar.generation
    };
    data.bookmark_root(BookmarkRoot::Toolbar, move |root| {
        let (Ok(root), Some(data)) = (root, profile_data::handle()) else {
            return;
        };
        data.bookmark_children(root, move |children| {
            let Ok(children) = children else {
                return;
            };
            with(|bookmarks| {
                {
                    let mut bar = bookmarks.bar.borrow_mut();
                    if bar.generation != generation {
                        return;
                    }
                    let known = bar
                        .nodes
                        .iter()
                        .zip(&bar.widths)
                        .filter_map(|(node, width)| {
                            Some(((node.title.clone(), is_folder(node)), (*width)?))
                        })
                        .collect::<HashMap<_, _>>();
                    let widths = children
                        .iter()
                        .take(MAX_BAR_BUTTONS)
                        .map(|node| known.get(&(node.title.clone(), is_folder(node))).copied())
                        .collect::<Vec<_>>();
                    let items = children
                        .iter()
                        .take(MAX_BAR_BUTTONS)
                        .map(|node| BookmarkBarItem {
                            title: node.title.as_str().into(),
                            is_folder: is_folder(node),
                            x: 0.0,
                            width: 0.0,
                            shown: false,
                        })
                        .collect::<Vec<_>>();
                    bar.root = Some(root);
                    bar.nodes = children;
                    bar.widths = widths;
                    bar.model.set_vec(items);
                }
                relayout(bookmarks);
            });
        });
    });
}

fn is_folder(node: &BookmarkNode) -> bool {
    node.kind == BookmarkKind::Folder
}

/// Places the buttons for the bar's width now.
fn relayout(bookmarks: &Rc<Bookmarks>) {
    let Some(window) = bookmarks.window.upgrade() else {
        return;
    };
    let mut bar = bookmarks.bar.borrow_mut();
    bar.relayout_pending = false;
    let widths = layout_widths(&bar.widths, bar.nodes.len());
    let layout = bar_layout(&widths, window.get_bookmarks_bar_available_width());
    for index in 0..bar.model.row_count() {
        let Some(mut item) = bar.model.row_data(index) else {
            continue;
        };
        let placed = layout
            .positions
            .get(index)
            .map(|x| (*x, bar.widths[index].unwrap_or(0.0)));
        let (x, width, shown) = placed.map_or((0.0, 0.0, false), |(x, width)| (x, width, true));
        if item.x != x || item.width != width || item.shown != shown {
            item.x = x;
            item.width = width;
            item.shown = shown;
            bar.model.set_row_data(index, item);
        }
    }
    window.set_bookmarks_bar_overflow(layout.overflow);
    bar.layout = layout;
}

/// Shows the innermost level of the popup, reading a folder's children first; opens the
/// popup at `x` when given.
fn load_menu(bookmarks: &Rc<Bookmarks>, open_at: Option<f32>) {
    let (level, generation) = {
        let mut bar = bookmarks.bar.borrow_mut();
        bar.menu_generation += 1;
        (bar.levels.last().cloned(), bar.menu_generation)
    };
    match level {
        None => {
            if let Some(window) = bookmarks.window.upgrade() {
                window.invoke_close_bookmarks_menu();
            }
        }
        Some(MenuLevel::Overflow) => {
            let nodes = {
                let bar = bookmarks.bar.borrow();
                bar.nodes[bar.layout.shown().min(bar.nodes.len())..].to_vec()
            };
            show_menu(bookmarks, nodes, open_at);
        }
        Some(MenuLevel::Folder { id, .. }) => {
            let Some(data) = profile_data::handle() else {
                return;
            };
            data.bookmark_children(id, move |children| {
                with(|bookmarks| {
                    if bookmarks.bar.borrow().menu_generation != generation {
                        return;
                    }
                    match children {
                        Ok(children) => show_menu(bookmarks, children, open_at),
                        Err(error) => log::warn!("a bookmark folder could not be read: {error}"),
                    }
                });
            });
        }
    }
}

fn show_menu(bookmarks: &Rc<Bookmarks>, nodes: Vec<BookmarkNode>, open_at: Option<f32>) {
    let Some(window) = bookmarks.window.upgrade() else {
        return;
    };
    let rows = {
        let mut bar = bookmarks.bar.borrow_mut();
        bar.entries = menu_entries(&bar.levels, nodes);
        bar.entries.iter().map(menu_row).collect::<Vec<_>>()
    };
    window.set_bookmarks_menu_rows(ModelRc::new(VecModel::from(rows)));
    if let Some(x) = open_at {
        window.invoke_show_bookmarks_menu(x);
    }
    window.window().request_redraw();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, title: &str, kind: BookmarkKind) -> BookmarkNode {
        BookmarkNode {
            id: BookmarkId(id),
            guid: String::new(),
            parent: None,
            position: 0,
            title: title.to_owned(),
            kind,
            added_at_ms: 0,
            modified_at_ms: 0,
        }
    }

    #[test]
    fn a_first_level_popup_has_no_back_row() {
        let entries = menu_entries(
            &[MenuLevel::Folder {
                id: BookmarkId(1),
                title: "News".into(),
            }],
            vec![node(2, "A", BookmarkKind::Separator)],
        );
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], MenuEntry::Node(_)));
    }

    #[test]
    fn a_nested_folder_starts_with_a_row_back_to_its_parent() {
        let levels = [
            MenuLevel::Overflow,
            MenuLevel::Folder {
                id: BookmarkId(1),
                title: "News".into(),
            },
        ];
        let entries = menu_entries(&levels, Vec::new());
        assert_eq!(
            entries,
            vec![
                MenuEntry::Back {
                    title: "More bookmarks".into()
                },
                MenuEntry::Empty
            ]
        );
        assert_eq!(menu_row(&entries[0]).kind, BookmarkRowKind::Back);
        assert_eq!(menu_row(&entries[1]).title, "Empty");
    }

    #[test]
    fn rows_carry_the_kind_and_url() {
        let row = menu_row(&MenuEntry::Node(node(
            1,
            "A",
            BookmarkKind::Bookmark {
                url: "https://a.test/".into(),
            },
        )));
        assert_eq!(
            (row.title.as_str(), row.url.as_str(), row.kind),
            ("A", "https://a.test/", BookmarkRowKind::Bookmark)
        );
    }

    #[test]
    fn the_bar_is_read_again_only_when_its_folder_changed() {
        let root = Some(BookmarkId(1));
        assert!(affected_by(root, Some(&[BookmarkId(5), BookmarkId(1)])));
        assert!(!affected_by(root, Some(&[BookmarkId(5)])));
        assert!(!affected_by(root, Some(&[])));
        assert!(
            affected_by(root, None),
            "no hint: anything may have changed"
        );
        assert!(
            affected_by(None, Some(&[BookmarkId(5)])),
            "the bar was never read"
        );
    }

    #[test]
    fn entries_beyond_the_buttons_made_force_the_overflow_button() {
        let widths = vec![Some(10.0); MAX_BAR_BUTTONS];
        let layout = bar_layout(&layout_widths(&widths, MAX_BAR_BUTTONS + 5), 1.0e6);
        assert!(layout.overflow);
        assert_eq!(layout.shown(), MAX_BAR_BUTTONS);
        let all = bar_layout(&layout_widths(&widths, MAX_BAR_BUTTONS), 1.0e6);
        assert!(!all.overflow);
    }
}
