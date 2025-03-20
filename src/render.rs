use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};

use crossterm::style::{Attribute, Attributes, Color};
use promkit::{pane::Pane, style::StyleBuilder, terminal::Terminal, text};
use tokio::sync::{Mutex, MutexGuard};

pub static EMPTY_PANE: LazyLock<Pane> = LazyLock::new(|| Pane::new(vec![], 0));

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct EditorIndex(pub usize, pub usize); // numerator, denominator

impl std::fmt::Display for EditorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({},{})", self.0, self.1)
    }
}

pub const HEAD_INDEX: EditorIndex = EditorIndex(1, 1);

impl PartialOrd for EditorIndex {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EditorIndex {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Comparing fractions: To compare a/b and c/d, compare ad and bc
        let left = (self.0 as u64) * (other.1 as u64);
        let right = (self.1 as u64) * (other.0 as u64);
        left.cmp(&right)
    }
}

impl EditorIndex {
    pub fn mediant(a: &EditorIndex, b: &EditorIndex) -> Self {
        // TODO: gcd to reduce the fraction
        Self(a.0 + b.0, a.1 + b.1)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum NotifyMessage {
    None,
    Error(String),
}

impl From<NotifyMessage> for text::State {
    fn from(val: NotifyMessage) -> Self {
        match val {
            NotifyMessage::None => text::State::default(),
            NotifyMessage::Error(message) => text::State {
                text: text::Text::from(message),
                style: StyleBuilder::new()
                    .fgc(Color::DarkRed)
                    .attrs(Attributes::from(Attribute::Bold))
                    .build(),
                ..Default::default()
            },
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PaneIndex {
    Notify,
    Editor(EditorIndex),
    Output,
}

impl PartialOrd for PaneIndex {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PaneIndex {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (PaneIndex::Notify, PaneIndex::Notify) => std::cmp::Ordering::Equal,
            (PaneIndex::Notify, _) => std::cmp::Ordering::Less,
            (_, PaneIndex::Notify) => std::cmp::Ordering::Greater,

            (PaneIndex::Output, PaneIndex::Output) => std::cmp::Ordering::Equal,
            (PaneIndex::Output, _) => std::cmp::Ordering::Greater,
            (_, PaneIndex::Output) => std::cmp::Ordering::Less,

            (PaneIndex::Editor(a), PaneIndex::Editor(b)) => a.cmp(b),
        }
    }
}
pub struct SharedRenderer(Arc<Mutex<Renderer>>);

impl SharedRenderer {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self(Arc::new(Mutex::new(Renderer::try_new()?))))
    }

    pub fn clone(&self) -> Self {
        Self(self.0.clone())
    }

    pub fn lock(&self) -> impl Future<Output = MutexGuard<'_, Renderer>> {
        self.0.lock()
    }
}

pub struct Renderer {
    terminal: Terminal,
    panes: BTreeMap<PaneIndex, Pane>,
}

impl Renderer {
    pub fn try_new() -> anyhow::Result<Self> {
        Ok(Self {
            terminal: Terminal {
                position: crossterm::cursor::position()?,
            },
            panes: BTreeMap::from([
                (PaneIndex::Notify, EMPTY_PANE.clone()),
                (PaneIndex::Editor(EditorIndex(1, 1)), EMPTY_PANE.clone()),
                (PaneIndex::Output, EMPTY_PANE.clone()),
            ]),
        })
    }

    pub fn update<I>(&mut self, items: I) -> &mut Self
    where
        I: IntoIterator<Item = (PaneIndex, Pane)>,
    {
        items.into_iter().for_each(|(index, pane)| {
            self.panes.insert(index, pane);
        });
        self
    }

    pub fn remove<I>(&mut self, items: I) -> &mut Self
    where
        I: IntoIterator<Item = PaneIndex>,
    {
        items.into_iter().for_each(|index| {
            self.panes.remove(&index);
        });
        self
    }

    pub fn render(&mut self) -> anyhow::Result<()> {
        self.terminal
            .draw(&self.panes.values().cloned().collect::<Vec<Pane>>())
    }
}
