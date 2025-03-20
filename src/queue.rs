use std::collections::VecDeque;

use promkit::{Cursor, PaneFactory, grapheme::StyledGraphemes, pane::Pane};

pub struct Queue {
    buf: Cursor<VecDeque<StyledGraphemes>>,
    capacity: usize,
}

impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: Cursor::new(VecDeque::with_capacity(capacity), 0, false),
            capacity,
        }
    }

    pub fn push(&mut self, item: StyledGraphemes) {
        if self.buf.contents().len() > self.capacity {
            self.buf.contents_mut().pop_front();
        }
        // Note: promkit::terminal::Terminal ignores empty items.
        // Therefore, it replace empty items with a null character.
        if item.is_empty() {
            self.buf.contents_mut().push_back("\0".into());
        } else {
            self.buf.contents_mut().push_back(item);
        }
    }
}

pub struct State {
    queue: Queue,
    capacity: usize,
}

impl State {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Queue::new(capacity),
            capacity,
        }
    }

    pub fn reset(&mut self) {
        self.queue = Queue::new(self.capacity);
    }

    pub fn push(&mut self, item: StyledGraphemes) {
        self.queue.push(item);
    }

    pub fn shift(&mut self, up: usize, down: usize) -> bool {
        self.queue.buf.shift(up, down)
    }
}

impl PaneFactory for State {
    fn create_pane(&self, width: u16, height: u16) -> Pane {
        Pane::new(
            self.queue
                .buf
                .contents()
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    *i >= self.queue.buf.position()
                        && *i < self.queue.buf.position() + height as usize
                })
                .fold((vec![], 0), |(mut acc, pos), (_, item)| {
                    let rows = item.matrixify(width as usize, height as usize, 0).0;
                    if pos < self.queue.buf.position() + height as usize {
                        acc.extend(rows);
                    }
                    (acc, pos + 1)
                })
                .0,
            0,
        )
    }
}
