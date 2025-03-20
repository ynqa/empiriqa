use std::{borrow::Borrow, fmt};

use crossterm::event::{MouseEvent, MouseEventKind};
use futures::StreamExt;
use promkit::crossterm::{
    self,
    event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers},
};
use tokio::{sync::mpsc, task::JoinHandle, time::Interval};

#[derive(Clone, Debug, PartialEq)]
pub enum Buffer {
    Key(Vec<char>),                        // (chars)
    VerticalCursor(usize, usize),          // (up, down)
    VerticalScroll(usize, usize),          // (up, down)
    HorizontalCursor(usize, usize),        // (left, right)
    HorizontalScroll(usize, usize),        // (left, right)
    Other(crossterm::event::Event, usize), // (event, count)
}

impl fmt::Display for Buffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Buffer::Key(chars) => write!(f, "Key({:?})", chars),
            Buffer::VerticalCursor(up, down) => write!(f, "VerticalCursor({}, {})", up, down),
            Buffer::VerticalScroll(up, down) => write!(f, "VerticalScroll({}, {})", up, down),
            Buffer::HorizontalCursor(left, right) => {
                write!(f, "HorizontalCursor({}, {})", left, right)
            }
            Buffer::HorizontalScroll(left, right) => {
                write!(f, "HorizontalScroll({}, {})", left, right)
            }
            Buffer::Other(event, count) => write!(f, "Other({:?}, {})", event, count),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Debounce {
    Resize(u16, u16), // (width, height)
}

impl fmt::Display for Debounce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Debounce::Resize(width, height) => write!(f, "Resize({}, {})", width, height),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EventStream {
    Buffer(Buffer),
    Debounce(Debounce),
}

impl fmt::Display for EventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventStream::Buffer(buffer) => write!(f, "{}", buffer),
            EventStream::Debounce(debounce) => write!(f, "{}", debounce),
        }
    }
}

pub struct EventOperator {
    pub background: JoinHandle<()>,
}

impl EventOperator {
    pub fn spawn(tx: mpsc::Sender<Vec<EventStream>>, mut interval: Interval) -> Self {
        Self {
            background: tokio::spawn(async move {
                let mut event_stream = crossterm::event::EventStream::new();
                let mut buf = vec![];

                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            let _ = tx.send(Self::operate(buf.drain(..))).await;
                        },
                        Some(Ok(event)) = event_stream.next() => {
                            buf.push(event);
                        },
                    }
                }
            }),
        }
    }

    fn operate<I, E>(events: I) -> Vec<EventStream>
    where
        I: IntoIterator<Item = E>,
        E: Borrow<crossterm::event::Event>,
    {
        let mut result = Vec::new();
        let mut current_chars = Vec::new();
        let mut current_vertical = (0, 0);
        let mut current_horizontal = (0, 0);
        let mut current_vertical_scroll = (0, 0);
        let mut current_horizontal_scroll = (0, 0);
        let mut current_others: Option<(crossterm::event::Event, usize)> = None;
        let mut last_resize: Option<(u16, u16)> = None;
        let mut resize_index: Option<usize> = None;

        for event_ref in events {
            let event = event_ref.borrow();
            match event {
                crossterm::event::Event::Resize(width, height) => {
                    Self::flush_all_buffers(
                        &mut result,
                        &mut current_chars,
                        &mut current_vertical,
                        &mut current_horizontal,
                        &mut current_vertical_scroll,
                        &mut current_horizontal_scroll,
                        &mut current_others,
                    );
                    last_resize = Some((*width, *height));
                    resize_index = Some(result.len());
                }
                event => {
                    if let Some(ch) = Self::extract_char(event) {
                        Self::flush_non_char_buffers(
                            &mut result,
                            &mut current_vertical,
                            &mut current_horizontal,
                            &mut current_vertical_scroll,
                            &mut current_horizontal_scroll,
                            &mut current_others,
                        );
                        current_chars.push(ch);
                    } else if let Some((up, down)) = Self::detect_vertical_direction(event) {
                        Self::flush_char_buffer(&mut result, &mut current_chars);
                        Self::flush_horizontal_buffer(&mut result, &mut current_horizontal);
                        Self::flush_vertical_scroll_buffer(
                            &mut result,
                            &mut current_vertical_scroll,
                        );
                        Self::flush_horizontal_scroll_buffer(
                            &mut result,
                            &mut current_horizontal_scroll,
                        );
                        Self::flush_others_buffer(&mut result, &mut current_others);
                        current_vertical.0 += up;
                        current_vertical.1 += down;
                    } else if let Some((up, down)) = Self::detect_vertical_scroll(event) {
                        Self::flush_char_buffer(&mut result, &mut current_chars);
                        Self::flush_vertical_buffer(&mut result, &mut current_vertical);
                        Self::flush_horizontal_buffer(&mut result, &mut current_horizontal);
                        Self::flush_horizontal_scroll_buffer(
                            &mut result,
                            &mut current_horizontal_scroll,
                        );
                        Self::flush_others_buffer(&mut result, &mut current_others);
                        current_vertical_scroll.0 += up;
                        current_vertical_scroll.1 += down;
                    } else if let Some((left, right)) = Self::detect_horizontal_direction(event) {
                        Self::flush_char_buffer(&mut result, &mut current_chars);
                        Self::flush_vertical_buffer(&mut result, &mut current_vertical);
                        Self::flush_vertical_scroll_buffer(
                            &mut result,
                            &mut current_vertical_scroll,
                        );
                        Self::flush_horizontal_scroll_buffer(
                            &mut result,
                            &mut current_horizontal_scroll,
                        );
                        Self::flush_others_buffer(&mut result, &mut current_others);
                        current_horizontal.0 += left;
                        current_horizontal.1 += right;
                    } else if let Some((left, right)) = Self::detect_horizontal_scroll(event) {
                        Self::flush_char_buffer(&mut result, &mut current_chars);
                        Self::flush_vertical_buffer(&mut result, &mut current_vertical);
                        Self::flush_vertical_scroll_buffer(
                            &mut result,
                            &mut current_vertical_scroll,
                        );
                        Self::flush_horizontal_buffer(&mut result, &mut current_horizontal);
                        Self::flush_others_buffer(&mut result, &mut current_others);
                        current_horizontal_scroll.0 += left;
                        current_horizontal_scroll.1 += right;
                    } else {
                        Self::flush_char_buffer(&mut result, &mut current_chars);
                        Self::flush_vertical_buffer(&mut result, &mut current_vertical);
                        Self::flush_vertical_scroll_buffer(
                            &mut result,
                            &mut current_vertical_scroll,
                        );
                        Self::flush_horizontal_buffer(&mut result, &mut current_horizontal);
                        Self::flush_horizontal_scroll_buffer(
                            &mut result,
                            &mut current_horizontal_scroll,
                        );

                        match &mut current_others {
                            Some((last_event, count)) if last_event == event => {
                                *count += 1;
                            }
                            _ => {
                                Self::flush_others_buffer(&mut result, &mut current_others);
                                current_others = Some((event.clone(), 1));
                            }
                        }
                    }
                }
            }
        }

        // Flush remaining buffers
        Self::flush_all_buffers(
            &mut result,
            &mut current_chars,
            &mut current_vertical,
            &mut current_horizontal,
            &mut current_vertical_scroll,
            &mut current_horizontal_scroll,
            &mut current_others,
        );

        // Add the last resize event if exists at the recorded index
        if let (Some((width, height)), Some(idx)) = (last_resize, resize_index) {
            result.insert(idx, EventStream::Debounce(Debounce::Resize(width, height)));
        }

        result
    }

    fn flush_all_buffers(
        result: &mut Vec<EventStream>,
        chars: &mut Vec<char>,
        vertical: &mut (usize, usize),
        horizontal: &mut (usize, usize),
        vertical_scroll: &mut (usize, usize),
        horizontal_scroll: &mut (usize, usize),
        others: &mut Option<(crossterm::event::Event, usize)>,
    ) {
        Self::flush_char_buffer(result, chars);
        Self::flush_vertical_buffer(result, vertical);
        Self::flush_horizontal_buffer(result, horizontal);
        Self::flush_vertical_scroll_buffer(result, vertical_scroll);
        Self::flush_horizontal_scroll_buffer(result, horizontal_scroll);
        Self::flush_others_buffer(result, others);
    }

    fn flush_char_buffer(result: &mut Vec<EventStream>, chars: &mut Vec<char>) {
        if !chars.is_empty() {
            result.push(EventStream::Buffer(Buffer::Key(chars.clone())));
            chars.clear();
        }
    }

    fn flush_vertical_buffer(result: &mut Vec<EventStream>, vertical: &mut (usize, usize)) {
        if *vertical != (0, 0) {
            result.push(EventStream::Buffer(Buffer::VerticalCursor(
                vertical.0, vertical.1,
            )));
            *vertical = (0, 0);
        }
    }

    fn flush_horizontal_buffer(result: &mut Vec<EventStream>, horizontal: &mut (usize, usize)) {
        if *horizontal != (0, 0) {
            result.push(EventStream::Buffer(Buffer::HorizontalCursor(
                horizontal.0,
                horizontal.1,
            )));
            *horizontal = (0, 0);
        }
    }

    fn flush_vertical_scroll_buffer(
        result: &mut Vec<EventStream>,
        vertical_scroll: &mut (usize, usize),
    ) {
        if *vertical_scroll != (0, 0) {
            result.push(EventStream::Buffer(Buffer::VerticalScroll(
                vertical_scroll.0,
                vertical_scroll.1,
            )));
            *vertical_scroll = (0, 0);
        }
    }

    fn flush_horizontal_scroll_buffer(
        result: &mut Vec<EventStream>,
        horizontal_scroll: &mut (usize, usize),
    ) {
        if *horizontal_scroll != (0, 0) {
            result.push(EventStream::Buffer(Buffer::HorizontalScroll(
                horizontal_scroll.0,
                horizontal_scroll.1,
            )));
            *horizontal_scroll = (0, 0);
        }
    }

    fn flush_others_buffer(
        result: &mut Vec<EventStream>,
        others: &mut Option<(crossterm::event::Event, usize)>,
    ) {
        if let Some((event, count)) = others.take() {
            result.push(EventStream::Buffer(Buffer::Other(event, count)));
        }
    }

    fn flush_non_char_buffers(
        result: &mut Vec<EventStream>,
        vertical: &mut (usize, usize),
        horizontal: &mut (usize, usize),
        vertical_scroll: &mut (usize, usize),
        horizontal_scroll: &mut (usize, usize),
        others: &mut Option<(crossterm::event::Event, usize)>,
    ) {
        Self::flush_vertical_buffer(result, vertical);
        Self::flush_horizontal_buffer(result, horizontal);
        Self::flush_vertical_scroll_buffer(result, vertical_scroll);
        Self::flush_horizontal_scroll_buffer(result, horizontal_scroll);
        Self::flush_others_buffer(result, others);
    }

    fn extract_char(event: &crossterm::event::Event) -> Option<char> {
        match event {
            crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            })
            | crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                modifiers: KeyModifiers::SHIFT,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            }) => Some(*ch),
            _ => None,
        }
    }

    fn detect_vertical_direction(event: &crossterm::event::Event) -> Option<(usize, usize)> {
        match event {
            crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Up, ..
            }) => Some((1, 0)),
            crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Down,
                ..
            }) => Some((0, 1)),
            _ => None,
        }
    }

    fn detect_vertical_scroll(event: &crossterm::event::Event) -> Option<(usize, usize)> {
        match event {
            crossterm::event::Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                ..
            }) => Some((1, 0)),
            crossterm::event::Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                ..
            }) => Some((0, 1)),
            _ => None,
        }
    }

    fn detect_horizontal_direction(event: &crossterm::event::Event) -> Option<(usize, usize)> {
        match event {
            crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Left,
                ..
            }) => Some((1, 0)),
            crossterm::event::Event::Key(KeyEvent {
                code: KeyCode::Right,
                ..
            }) => Some((0, 1)),
            _ => None,
        }
    }

    fn detect_horizontal_scroll(event: &crossterm::event::Event) -> Option<(usize, usize)> {
        match event {
            crossterm::event::Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollLeft,
                ..
            }) => Some((1, 0)),
            crossterm::event::Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollRight,
                ..
            }) => Some((0, 1)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod operate {
        use super::*;

        #[test]
        fn test() {
            // Input:
            // 'a', 'B', 'c', Resize(128, 128), Resize(256, 256),
            // Up, Down, Up, ScrollDown, ScrollUp, Left, Right, Left,
            // Ctrl+f, Ctrl+f, Ctrl+f, Ctrl+d,
            // Up, Resize(64, 64), 'd'
            let events = vec![
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('a'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('B'),
                    modifiers: KeyModifiers::SHIFT,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Resize(128, 128),
                crossterm::event::Event::Resize(256, 256),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    modifiers: KeyModifiers::NONE,
                    row: 0,
                    column: 0,
                }),
                crossterm::event::Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollUp,
                    modifiers: KeyModifiers::NONE,
                    row: 0,
                    column: 0,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Right,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Left,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('f'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('d'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Up,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
                crossterm::event::Event::Resize(64, 64),
                crossterm::event::Event::Key(KeyEvent {
                    code: KeyCode::Char('d'),
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: KeyEventState::NONE,
                }),
            ];

            let expected = vec![
                EventStream::Buffer(Buffer::Key(vec!['a', 'B', 'c'])),
                EventStream::Buffer(Buffer::VerticalCursor(2, 1)),
                EventStream::Buffer(Buffer::VerticalScroll(1, 1)),
                EventStream::Buffer(Buffer::HorizontalCursor(2, 1)),
                EventStream::Buffer(Buffer::Other(
                    crossterm::event::Event::Key(KeyEvent {
                        code: KeyCode::Char('f'),
                        modifiers: KeyModifiers::CONTROL,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }),
                    3,
                )),
                EventStream::Buffer(Buffer::Other(
                    crossterm::event::Event::Key(KeyEvent {
                        code: KeyCode::Char('d'),
                        modifiers: KeyModifiers::CONTROL,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }),
                    1,
                )),
                EventStream::Buffer(Buffer::VerticalCursor(1, 0)),
                EventStream::Debounce(Debounce::Resize(64, 64)),
                EventStream::Buffer(Buffer::Key(vec!['d'])),
            ];

            assert_eq!(EventOperator::operate(&events), expected);
        }
    }
}
