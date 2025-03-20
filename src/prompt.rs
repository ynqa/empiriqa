use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::bail;
use crossterm::{
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers},
    style::{Attribute, Color},
};
use promkit::{PaneFactory, pane::Pane, style::StyleBuilder, text_editor};
use tokio::{
    sync::{Mutex, broadcast, mpsc},
    task::JoinHandle,
};

use crate::{
    operator::{Buffer, Debounce, EventStream},
    render::{EditorIndex, HEAD_INDEX, NotifyMessage, PaneIndex, SharedRenderer},
};

fn edit(event: &EventStream, editor: &mut text_editor::State) {
    match event {
        // Move cursor.
        EventStream::Buffer(Buffer::HorizontalCursor(left, right)) => {
            editor.texteditor.shift(*left, *right);
        }
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            _,
        )) => {
            editor.texteditor.move_to_head();
        }
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('e'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            _,
        )) => {
            editor.texteditor.move_to_tail();
        }

        // Move cursor to the nearest character.
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::ALT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            times,
        )) => {
            for _ in 0..*times {
                editor
                    .texteditor
                    .move_to_previous_nearest(&editor.word_break_chars);
            }
        }
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::ALT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            times,
        )) => {
            for _ in 0..*times {
                editor
                    .texteditor
                    .move_to_next_nearest(&editor.word_break_chars);
            }
        }

        // Erase char(s).
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            times,
        )) => {
            for _ in 0..*times {
                editor.texteditor.erase();
            }
        }
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            _,
        )) => {
            editor.texteditor.erase_all();
        }

        // Erase to the nearest character.
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            times,
        )) => {
            for _ in 0..*times {
                editor
                    .texteditor
                    .erase_to_previous_nearest(&editor.word_break_chars);
            }
        }
        EventStream::Buffer(Buffer::Other(
            Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::ALT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }),
            times,
        )) => {
            for _ in 0..*times {
                editor
                    .texteditor
                    .erase_to_next_nearest(&editor.word_break_chars);
            }
        }

        // Input char.
        EventStream::Buffer(Buffer::Key(chars)) => match editor.edit_mode {
            text_editor::Mode::Insert => editor.texteditor.insert_chars(chars),
            text_editor::Mode::Overwrite => editor.texteditor.overwrite_chars(chars),
        },

        _ => {}
    }
}

#[derive(Clone)]
pub struct EditorTheme {
    pub prefix: String,
    pub prefix_fg_color: Color,
    pub active_char_bg_color: Color,
    pub word_break_chars: HashSet<char>,
}

struct Editor {
    state: text_editor::State,
    ignore: bool,
}

impl From<text_editor::State> for Editor {
    fn from(state: text_editor::State) -> Self {
        Self {
            state,
            ignore: false,
        }
    }
}

impl Editor {
    fn create_pane(&self, width: u16, height: u16) -> Pane {
        self.state.create_pane(width, height)
    }
}

struct EditorMap(BTreeMap<EditorIndex, Editor>);

enum Direction {
    Up(usize),
    Down(usize),
}

impl Direction {
    fn distance(&self) -> usize {
        match self {
            Self::Up(up) => *up,
            Self::Down(down) => *down,
        }
    }
}

impl EditorMap {
    fn from(state: text_editor::State) -> Self {
        Self(BTreeMap::from_iter([(
            HEAD_INDEX.clone(),
            Editor::from(state),
        )]))
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, index: &EditorIndex) -> Option<&Editor> {
        self.0.get(index)
    }

    fn get_mut(&mut self, index: &EditorIndex) -> Option<&mut Editor> {
        self.0.get_mut(index)
    }

    fn insert(&mut self, index: EditorIndex, state: text_editor::State) -> Option<Editor> {
        self.0.insert(index, Editor::from(state))
    }

    fn pop_last(&mut self) -> Option<(EditorIndex, Editor)> {
        self.0.pop_last()
    }

    fn iter(&self) -> impl Iterator<Item = (&EditorIndex, &Editor)> {
        self.0.iter()
    }

    fn remove(&mut self, index: &EditorIndex) -> Option<Editor> {
        self.0.remove(index)
    }

    fn values(&self) -> impl Iterator<Item = &Editor> {
        self.0.values()
    }

    fn last_index(&self) -> Option<&EditorIndex> {
        self.0.keys().last()
    }

    fn contains_key(&self, index: &EditorIndex) -> bool {
        self.0.contains_key(index)
    }

    fn is_last(&self, index: &EditorIndex) -> bool {
        if let Some(last) = self.0.keys().last() {
            last.0 == index.0 && last.1 == index.1
        } else {
            false
        }
    }

    fn new_index(&self, index: &EditorIndex) -> anyhow::Result<EditorIndex> {
        if self.is_last(index) {
            // If this is the last index, create a new index that is greater
            // A simple way is to add 1 to the numerator while keeping the denominator
            Ok(EditorIndex(index.0 + 1, index.1))
        } else {
            Ok(EditorIndex::mediant(
                index,
                &self.seek_index(index, Direction::Down(1))?,
            ))
        }
    }

    fn shift_index(
        &self,
        index: &EditorIndex,
        up: usize,
        down: usize,
    ) -> anyhow::Result<EditorIndex> {
        match up.cmp(&down) {
            Ordering::Less => self.seek_index(index, Direction::Down(down.saturating_sub(up))),
            Ordering::Greater => self.seek_index(index, Direction::Up(up.saturating_sub(down))),
            Ordering::Equal => Ok(index.clone()),
        }
    }

    fn seek_index(&self, index: &EditorIndex, direction: Direction) -> anyhow::Result<EditorIndex> {
        if !self.contains_key(index) {
            bail!("{} not found", index);
        }

        let mut iter = match direction {
            Direction::Up(_) => {
                Box::new(
                    self.0
                        .keys()
                        .rev()
                        .skip_while(|k| !(k.0 == index.0 && k.1 == index.1))
                        // Skip the current index
                        .skip(1),
                ) as Box<dyn Iterator<Item = &EditorIndex>>
            }
            Direction::Down(_) => {
                Box::new(
                    self.0
                        .keys()
                        .skip_while(|k| !(k.0 == index.0 && k.1 == index.1))
                        // Skip the current index
                        .skip(1),
                ) as Box<dyn Iterator<Item = &EditorIndex>>
            }
        };

        let (mut cur, mut remaining) = (index.clone(), direction.distance());

        while let Some(next) = iter.next() {
            if remaining == 0 {
                break;
            }

            cur = next.clone();
            remaining -= 1;
        }

        Ok(cur)
    }
}

pub struct Prompt {
    // TODO: reconsider whether mutex is necessary only for get_all_texts
    shared_editors: Arc<Mutex<EditorMap>>,
    pub background: JoinHandle<()>,
}

impl Prompt {
    pub fn spawn(
        mut rx: broadcast::Receiver<EventStream>,
        notify_tx: mpsc::Sender<NotifyMessage>,
        themes: (EditorTheme, EditorTheme), // (head, pipe)
        init_terminal_shape: (u16, u16),
        shared_renderer: SharedRenderer,
    ) -> Self {
        let shared_editors = Arc::new(Mutex::new(EditorMap::from(text_editor::State {
            prefix: themes.0.prefix.clone(),
            prefix_style: StyleBuilder::new().fgc(themes.0.prefix_fg_color).build(),
            active_char_style: StyleBuilder::new()
                .bgc(themes.0.active_char_bg_color)
                .build(),
            word_break_chars: themes.0.word_break_chars.clone(),
            ..Default::default()
        })));

        let background = {
            let mut terminal_shape = init_terminal_shape;
            let shared_editors = shared_editors.clone();

            tokio::spawn(async move {
                let mut cur_index = HEAD_INDEX.clone();

                // Initial renderings
                {
                    let (editors, mut renderer) =
                        tokio::join!(shared_editors.lock(), shared_renderer.lock());

                    let _ = renderer
                        .update(editors.iter().map(|(index, editor)| {
                            (
                                PaneIndex::Editor(index.clone()),
                                editor.create_pane(terminal_shape.0, terminal_shape.1),
                            )
                        }))
                        .render();
                }

                loop {
                    if let Ok(event) = rx.recv().await {
                        match event {
                            EventStream::Debounce(Debounce::Resize(width, height)) => {
                                terminal_shape = (width, height);

                                let (mut editors, mut renderer) =
                                    tokio::join!(shared_editors.lock(), shared_renderer.lock());

                                // Resize the editors also
                                // Note to consider the notify and output panes...
                                if height < editors.len() as u16 + 2 {
                                    let removals = {
                                        let times =
                                            (editors.len() + 2).saturating_sub(height as usize);
                                        Self::pop_editors(&mut editors, times)
                                    };
                                    renderer.remove(removals.into_iter().map(PaneIndex::Editor));

                                    // Update the current index
                                    cur_index = HEAD_INDEX.clone();
                                    // Change theme because of switching focus
                                    Self::switch_theme(&mut editors, None, &cur_index, &themes);
                                }

                                renderer.update(editors.iter().map(|(index, editor)| {
                                    (
                                        PaneIndex::Editor(index.clone()),
                                        editor.create_pane(terminal_shape.0, terminal_shape.1),
                                    )
                                }));
                            }
                            EventStream::Buffer(Buffer::Other(
                                Event::Key(KeyEvent {
                                    code: KeyCode::Char('b'),
                                    modifiers: KeyModifiers::CONTROL,
                                    kind: KeyEventKind::Press,
                                    state: KeyEventState::NONE,
                                }),
                                times,
                            )) => {
                                let mut new_index = cur_index.clone();
                                let mut inserts = HashSet::from([new_index.clone()]);

                                let mut editors = shared_editors.lock().await;
                                // Insert new editors
                                for _ in 0..times {
                                    // 2 represents the notify and output panes
                                    if editors.len() >= terminal_shape.1.saturating_sub(2) as usize
                                    {
                                        let _ = notify_tx
                                            .send(NotifyMessage::Error(String::from(
                                                "Cannot create more editors",
                                            )))
                                            .await;
                                        break;
                                    }
                                    new_index =
                                        Self::insert_editor(&new_index, &mut editors, &themes.1);
                                    inserts.insert(new_index.clone());
                                }
                                // Change theme because of switching focus
                                Self::switch_theme(
                                    &mut editors,
                                    Some(&cur_index),
                                    &new_index,
                                    &themes,
                                );
                                // Update changes for rendering
                                shared_renderer.lock().await.update(inserts.into_iter().map(
                                    |index| {
                                        (
                                            PaneIndex::Editor(index.clone()),
                                            editors
                                                .get(&index)
                                                .unwrap()
                                                .create_pane(terminal_shape.0, terminal_shape.1),
                                        )
                                    },
                                ));
                                // Update the current index
                                cur_index = new_index;
                            }
                            EventStream::Buffer(Buffer::Other(
                                Event::Key(KeyEvent {
                                    code: KeyCode::Char('d'),
                                    modifiers: KeyModifiers::CONTROL,
                                    kind: KeyEventKind::Press,
                                    state: KeyEventState::NONE,
                                }),
                                times,
                            )) => {
                                let mut prev_index = cur_index.clone();
                                let mut removals = HashSet::new();

                                {
                                    let mut editors = shared_editors.lock().await;
                                    // Remove editors
                                    for _ in 0..times {
                                        // Early return if the head editor is removed
                                        if prev_index == HEAD_INDEX {
                                            break;
                                        }
                                        removals.insert(prev_index.clone());
                                        prev_index = Self::remove_editor(&prev_index, &mut editors);
                                    }
                                    // Change theme because of switching focus
                                    Self::switch_theme(&mut editors, None, &prev_index, &themes);
                                }

                                // Update changes for rendering
                                {
                                    let mut renderer = shared_renderer.lock().await;
                                    let _ = renderer
                                        .remove(removals.into_iter().map(PaneIndex::Editor))
                                        .update([(
                                            PaneIndex::Editor(prev_index.clone()),
                                            shared_editors
                                                .lock()
                                                .await
                                                .get(&prev_index)
                                                .unwrap()
                                                .create_pane(terminal_shape.0, terminal_shape.1),
                                        )]);
                                }

                                // Update the current index
                                cur_index = prev_index;
                            }
                            EventStream::Buffer(Buffer::Other(
                                Event::Key(KeyEvent {
                                    code: KeyCode::Char('x'),
                                    modifiers: KeyModifiers::CONTROL,
                                    kind: KeyEventKind::Press,
                                    state: KeyEventState::NONE,
                                }),
                                times,
                            )) => {
                                if times % 2 != 0 {
                                    let mut editors = shared_editors.lock().await;
                                    let cur_editor = editors.get_mut(&cur_index).unwrap();
                                    cur_editor.ignore = !cur_editor.ignore;
                                    cur_editor
                                        .state
                                        .prefix_style
                                        .attributes
                                        .toggle(Attribute::CrossedOut);
                                    cur_editor
                                        .state
                                        .active_char_style
                                        .attributes
                                        .toggle(Attribute::CrossedOut);
                                    cur_editor
                                        .state
                                        .inactive_char_style
                                        .attributes
                                        .toggle(Attribute::CrossedOut);
                                    shared_renderer.lock().await.update(vec![(
                                        PaneIndex::Editor(cur_index.clone()),
                                        cur_editor.create_pane(terminal_shape.0, terminal_shape.1),
                                    )]);
                                }
                            }
                            EventStream::Buffer(Buffer::VerticalCursor(up, down)) => {
                                let mut editors = shared_editors.lock().await;
                                // Move cursor up or down
                                let next_index = editors.shift_index(&cur_index, up, down).unwrap();
                                // Change theme because of switching focus
                                Self::switch_theme(
                                    &mut editors,
                                    Some(&cur_index),
                                    &next_index,
                                    &themes,
                                );
                                // Update changes for rendering
                                shared_renderer.lock().await.update(vec![
                                    (
                                        PaneIndex::Editor(cur_index.clone()),
                                        editors
                                            .get(&cur_index)
                                            .unwrap()
                                            .create_pane(terminal_shape.0, terminal_shape.1),
                                    ),
                                    (
                                        PaneIndex::Editor(next_index.clone()),
                                        editors
                                            .get(&next_index)
                                            .unwrap()
                                            .create_pane(terminal_shape.0, terminal_shape.1),
                                    ),
                                ]);
                                // Update the current index
                                cur_index = next_index;
                            }
                            event => {
                                let mut editors = shared_editors.lock().await;
                                edit(&event, &mut editors.get_mut(&cur_index).unwrap().state);
                                shared_renderer.lock().await.update(vec![(
                                    PaneIndex::Editor(cur_index.clone()),
                                    editors
                                        .get(&cur_index)
                                        .unwrap()
                                        .create_pane(terminal_shape.0, terminal_shape.1),
                                )]);
                            }
                        };

                        let _ = shared_renderer.lock().await.render();
                    }
                }
            })
        };

        Self {
            shared_editors,
            background,
        }
    }

    pub async fn get_all_texts(&mut self) -> Vec<String> {
        self.shared_editors
            .lock()
            .await
            .values()
            .filter(|editor| !editor.ignore)
            .map(|editor| editor.state.texteditor.text_without_cursor().to_string())
            .filter(|cmd| !cmd.trim().is_empty())
            .collect()
    }

    fn insert_editor(
        cur_index: &EditorIndex,
        editors: &mut EditorMap,
        theme: &EditorTheme,
    ) -> EditorIndex {
        let new_index = editors.new_index(cur_index).unwrap();
        editors.insert(
            new_index.clone(),
            text_editor::State {
                prefix: theme.prefix.clone(),
                prefix_style: StyleBuilder::new().fgc(theme.prefix_fg_color).build(),
                active_char_style: StyleBuilder::new().bgc(theme.active_char_bg_color).build(),
                word_break_chars: theme.word_break_chars.clone(),
                ..Default::default()
            },
        );
        new_index
    }

    fn pop_editors(editors: &mut EditorMap, times: usize) -> Vec<EditorIndex> {
        let mut popped = vec![];
        for _ in 0..times {
            if editors.last_index() == Some(&HEAD_INDEX) {
                return popped;
            }
            popped.push(editors.pop_last().unwrap().0);
        }
        popped
    }

    fn remove_editor(cur_index: &EditorIndex, editors: &mut EditorMap) -> EditorIndex {
        // Do not remove the head editor
        if cur_index == &HEAD_INDEX {
            return cur_index.clone();
        }

        // Note that we're moving the index to the previous one
        // because the given index is the focused editor.
        // If in the future we need to remove a non-focused editor,
        // this operation would be unnecessary.
        let prev_index = editors.seek_index(cur_index, Direction::Up(1)).unwrap();

        editors.remove(cur_index);

        prev_index
    }

    fn switch_theme(
        editors: &mut EditorMap,
        defocus_index: Option<&EditorIndex>,
        focus_index: &EditorIndex,
        themes: &(EditorTheme, EditorTheme), // (head, pipe)
    ) {
        if Some(focus_index) == defocus_index {
            return;
        }

        if let Some(defocus_index) = defocus_index {
            let defocus = editors.get_mut(defocus_index).unwrap();
            defocus.state.prefix_style.attributes.set(Attribute::Dim);
            defocus
                .state
                .inactive_char_style
                .attributes
                .set(Attribute::Dim);
            defocus.state.active_char_style.background_color = None;
            defocus
                .state
                .active_char_style
                .attributes
                .set(Attribute::Dim);
        }

        let focus = editors.get_mut(focus_index).unwrap();
        let theme = match focus_index {
            &HEAD_INDEX => themes.0.clone(),
            _ => themes.1.clone(),
        };
        focus.state.prefix_style.attributes.unset(Attribute::Dim);
        focus
            .state
            .inactive_char_style
            .attributes
            .unset(Attribute::Dim);
        focus.state.active_char_style.background_color = Some(theme.active_char_bg_color);
        focus
            .state
            .active_char_style
            .attributes
            .unset(Attribute::Dim);
    }
}
