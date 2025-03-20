use std::{collections::HashSet, time::Duration};

use chrono::Local;
use clap::Parser;
use crossterm::{
    self,
    event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers},
    style::Color,
};
use promkit::{PaneFactory, grapheme::StyledGraphemes, text};
use tokio::sync::{broadcast, mpsc};

mod operator;
mod pipeline;
mod prompt;
use prompt::EditorTheme;
mod queue;
mod render;
use render::NotifyMessage;

use crate::{
    operator::{Buffer, EventOperator, EventStream},
    pipeline::Pipeline,
    prompt::Prompt,
    render::{PaneIndex, SharedRenderer},
};

/// Laboratory for pipeline construction with feedback
#[derive(Parser)]
#[command(name = "epiq", version)]
pub struct Args {
    #[arg(
        long,
        default_value = "1000",
        help = "Set the size of the output queue",
        long_help = "Sets the size of the queue that holds output from the pipeline. \
                    A larger value allows storing more output history, \
                    but increases memory usage."
    )]
    output_queue_size: usize,

    #[arg(
        long,
        default_value = "32",
        help = "Event processing aggregation interval (milliseconds)",
        long_help = "Specifies the time boundary in milliseconds for aggregating event operations \
                    (such as key inputs and mouse operations). Multiple events occurring within \
                    this time frame are processed together (e.g., debounce, buffering). \
                    Setting a smaller value improves responsiveness, but may cause internal \
                    processing to bottleneck when a large number of events are issued during \
                    scrolling or pasting. Setting an appropriate value enables efficient \
                    operation by buffering and processing events in batches."
    )]
    event_operate_interval: u64,

    #[arg(
        long,
        default_value = "10",
        help = "Output rendering interval (milliseconds)",
        long_help = "Specifies the interval in milliseconds for rendering pipeline output to the screen. \
                    Setting a smaller value increases the frequency of display updates, \
                    but may cause screen flickering due to frequent rendering operations."
    )]
    output_render_interval: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::cursor::Hide,
        crossterm::event::EnableMouseCapture,
    )?;

    let mut enable_mouse_capture = true;
    let mut cur_pipeline: Option<Pipeline> = None;
    let (event_tx, mut event_rx) = mpsc::channel(1);
    let event_operator = EventOperator::spawn(
        event_tx,
        tokio::time::interval(Duration::from_millis(args.event_operate_interval)),
    );
    let shared_renderer = SharedRenderer::try_new()?;
    let (broadcast_event_tx, _) = broadcast::channel(1);
    let (broadcast_reset_tx, _) = broadcast::channel(1);

    let (notify_tx, notify_rx) = mpsc::channel(1);
    let notify_renderer = shared_renderer.clone();
    let notify_stream = tokio::spawn(async move {
        notify_stream(text::State::default(), notify_rx, notify_renderer).await
    });

    let (output_tx, output_rx) = mpsc::channel(1);
    let output_renderer = shared_renderer.clone();
    let output_event_subscriber = broadcast_event_tx.subscribe();
    let output_reset_subscriber = broadcast_reset_tx.subscribe();
    let output_stream = tokio::spawn(async move {
        output_stream(
            queue::State::new(args.output_queue_size),
            output_rx,
            output_event_subscriber,
            output_reset_subscriber,
            output_renderer,
            Duration::from_millis(args.output_render_interval),
        )
        .await
    });

    let mut prompt = Prompt::spawn(
        broadcast_event_tx.subscribe(),
        notify_tx.clone(),
        // TODO: Configurable theme
        (
            // Head theme
            EditorTheme {
                prefix: String::from("❯❯ "),
                prefix_fg_color: Color::DarkGreen,
                active_char_bg_color: Color::DarkCyan,
                word_break_chars: HashSet::from(['.', '|', '(', ')', '[', ']']),
            },
            // Pipe theme
            EditorTheme {
                prefix: String::from("❚ "),
                prefix_fg_color: Color::DarkYellow,
                active_char_bg_color: Color::DarkCyan,
                word_break_chars: HashSet::from(['.', '|', '(', ')', '[', ']']),
            },
        ),
        crossterm::terminal::size()?,
        shared_renderer.clone(),
    );

    'outer: while let Some(events) = event_rx.recv().await {
        for event in events {
            match event {
                EventStream::Buffer(Buffer::Other(
                    Event::Key(KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }),
                    _,
                )) => break 'outer,
                // There is no way to capture ONLY mouse scroll events,
                // so, toggle enabling and disabling of capturing all mouse events with Esc.
                // https://github.com/crossterm-rs/crossterm/issues/640
                EventStream::Buffer(Buffer::Other(
                    Event::Key(KeyEvent {
                        code: KeyCode::Esc,
                        modifiers: KeyModifiers::NONE,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }),
                    times,
                )) => {
                    if times % 2 != 0 {
                        enable_mouse_capture = !enable_mouse_capture;
                        if enable_mouse_capture {
                            crossterm::execute!(
                                std::io::stdout(),
                                crossterm::event::EnableMouseCapture,
                            )?;
                        } else {
                            crossterm::execute!(
                                std::io::stdout(),
                                crossterm::event::DisableMouseCapture,
                            )?;
                        }
                    }
                }
                EventStream::Buffer(Buffer::Other(
                    Event::Key(KeyEvent {
                        code: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                        kind: KeyEventKind::Press,
                        state: KeyEventState::NONE,
                    }),
                    _,
                )) => {
                    // First of all, abort the current command if it is running.
                    if let Some(ref mut pipeline) = cur_pipeline {
                        pipeline.abort_all();
                        broadcast_reset_tx.send(())?;
                        let _ = notify_tx.send(NotifyMessage::None).await;
                    }

                    match Pipeline::spawn(prompt.get_all_texts().await, output_tx.clone()) {
                        Ok(pipeline) => {
                            cur_pipeline = Some(pipeline);
                        }
                        Err(e) => {
                            let _ = notify_tx
                                .send(NotifyMessage::Error(format!(
                                    "Cannot spawn commands: {:?}",
                                    e
                                )))
                                .await;
                        }
                    }
                }
                event => {
                    broadcast_event_tx.send(event)?;
                }
            }
        }
    }

    event_operator.background.abort();
    if let Some(mut pipeline) = cur_pipeline {
        pipeline.abort_all();
    }
    prompt.background.abort();
    output_stream.abort();
    notify_stream.abort();

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        std::io::stdout(),
        crossterm::cursor::Show,
        crossterm::event::DisableMouseCapture,
    )?;
    Ok(())
}

async fn notify_stream(
    mut text: text::State,
    mut stream: mpsc::Receiver<NotifyMessage>,
    shared_renderer: SharedRenderer,
) {
    while let Some(message) = stream.recv().await {
        text.replace(message.into());

        let mut renderer = shared_renderer.lock().await;
        if let Ok((width, height)) = crossterm::terminal::size() {
            let _ = renderer
                .update([(PaneIndex::Notify, text.create_pane(width, height))])
                .render();
        }
    }
}

async fn output_stream(
    mut queue: queue::State,
    mut stdout_stream: mpsc::Receiver<String>,
    mut event_stream: broadcast::Receiver<EventStream>,
    mut reset: broadcast::Receiver<()>,
    shared_renderer: SharedRenderer,
    render_interval: Duration,
) {
    let mut delay = tokio::time::interval(render_interval);
    let mut last_modified_time = Local::now();
    let mut last_render_time = Local::now();

    loop {
        tokio::select! {
            _ = reset.recv() => {
                queue.reset();
                last_modified_time = Local::now();
                last_render_time = Local::now();

                let _ = shared_renderer.lock().await.remove([
                    PaneIndex::Output,
                ]).render();
            },
            _ = delay.tick() => {
                if last_modified_time > last_render_time {
                    if let Ok((width, height)) = crossterm::terminal::size() {
                        let _ = shared_renderer.lock().await.update([
                            (PaneIndex::Output, queue.create_pane(width, height)),
                        ]).render();

                        last_render_time = Local::now();
                    }
                }
            },
            Ok(EventStream::Buffer(Buffer::VerticalScroll(up, down))) = event_stream.recv() => {
                let shifted = queue.shift(up, down);
                if shifted {
                    last_modified_time = Local::now();
                }
            },
            maybe_line = stdout_stream.recv() => {
                match maybe_line {
                    Some(line) => {
                        queue.push(StyledGraphemes::from(line));
                        last_modified_time = Local::now();
                    }
                    None => {
                        break;
                    }
                }
            },
        }
    }
}
