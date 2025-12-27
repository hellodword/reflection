use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

mod doc;
mod node;

use anyhow::{Result, anyhow};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{execute, terminal};
use doc::document::{Document, DocumentId};
use doc::identity::PrivateKey;
use doc::service::Service;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the file containing the document ID
    #[arg(long)]
    doc_id_file: PathBuf,

    /// Optional data directory for service persistence
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokio::time::Interval;
use tracing::error;

struct App {
    service: Service,
    document: Document,
    document_id: DocumentId,
    text: String,
    cursor: usize,
    status: String,
}

impl App {
    async fn new(data_dir: Option<PathBuf>, doc_id_file: PathBuf) -> Result<Self> {
        let private_key = PrivateKey::new();
        let service = Service::new(&private_key, data_dir.as_deref());
        service.startup().await?;

        let (document_id, initial_status) = resolve_document_id(doc_id_file)?;
        let document = service.join_document(&document_id);
        document.subscribe().await;

        let text = document.text();
        let cursor = text.len();

        Ok(Self {
            service,
            document,
            document_id,
            text,
            cursor,
            status: initial_status,
        })
    }

    fn handle_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Char('c') if self.is_ctrl_c() => {}
            KeyCode::Char(ch) => {
                if let Err(err) = self
                    .document
                    .insert_text(self.cursor as i32, &ch.to_string())
                {
                    self.status = format!("Write failed: {err}");
                    return;
                }
                self.cursor += ch.len_utf8();
                self.text = self.document.text();
            }
            KeyCode::Enter => {
                if let Err(err) = self.document.insert_text(self.cursor as i32, "\n") {
                    self.status = format!("Write failed: {err}");
                    return;
                }
                self.cursor += 1;
                self.text = self.document.text();
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let start = self.cursor - 1;
                    if let Err(err) = self.document.delete_range(start as i32, self.cursor as i32) {
                        self.status = format!("Delete failed: {err}");
                        return;
                    }
                    self.cursor = start;
                    self.text = self.document.text();
                }
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor < self.text.len() {
                    self.cursor += 1;
                }
            }
            _ => {}
        }
    }

    fn is_ctrl_c(&self) -> bool {
        // crossterm already handles Ctrl+C as KeyCode::Char('c') with modifier, but
        // we exit elsewhere by matching Event::Key with modifiers.
        false
    }

    fn refresh_from_doc(&mut self) {
        let latest = self.document.text();
        if latest.len() < self.cursor {
            self.cursor = latest.len();
        }
        self.text = latest;
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    setup_logging();

    let args = Args::parse();
    let mut app = App::new(args.data_dir, args.doc_id_file).await?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let mut ticker = tokio::time::interval(Duration::from_millis(200));

    let res = run_loop(&mut terminal, &mut app, &mut ticker).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        terminal::LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    app.service.shutdown().await;

    if let Err(err) = res {
        error!("Error on exit: {err}");
    }

    Ok(())
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    ticker: &mut Interval,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        tokio::select! {
            _ = ticker.tick() => {
                app.refresh_from_doc();
            }
            event = read_event() => {
                let ev = event?;
                if let Some(ev) = ev {
                    match ev {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            if key.code == KeyCode::Char('q') || key.code == KeyCode::Esc {
                                return Ok(());
                            }
                            app.handle_input(key.code);
                        }
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                }
            }
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
            .as_ref(),
        )
        .split(frame.size());

    let header = Paragraph::new(vec![Line::from(vec![
        Span::styled("Reflection TUI", Style::default().fg(Color::Cyan)),
        Span::raw("  |  Doc ID: "),
        Span::styled(
            format!("{}", app.document_id),
            Style::default().fg(Color::Yellow),
        ),
    ])]);

    let body = Paragraph::new(app.text.as_str())
        .block(Block::default().borders(Borders::ALL).title("Plain Text"));

    let footer = Paragraph::new(Line::from(vec![
        Span::raw("Type text, Backspace deletes, q exits"),
        Span::raw("  |  Status: "),
        Span::styled(app.status.as_str(), Style::default().fg(Color::Green)),
    ]));

    frame.render_widget(header, chunks[0]);
    frame.render_widget(body, chunks[1]);
    frame.render_widget(footer, chunks[2]);
}

async fn read_event() -> Result<Option<Event>> {
    let polled = tokio::task::spawn_blocking(|| event::poll(Duration::from_millis(50))).await??;
    if !polled {
        return Ok(None);
    }
    let ev = tokio::task::spawn_blocking(event::read).await??;
    Ok(Some(ev))
}

fn setup_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
}

fn resolve_document_id(doc_id_file: PathBuf) -> Result<(DocumentId, String)> {
    let raw = fs::read_to_string(&doc_id_file).unwrap_or_default();
    let hex = raw.trim();

    if hex.is_empty() {
        let id = DocumentId::new();
        if let Some(parent) = doc_id_file.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&doc_id_file, id.to_string())?;
        return Ok((
            id,
            format!(
                "Created new document and wrote to doc_id_file: {}",
                doc_id_file.display()
            ),
        ));
    }

    let id = DocumentId::from_hex(hex)
        .map_err(|e| anyhow!("Failed to parse doc_id_file contents: {e}"))?;
    Ok((
        id,
        format!(
            "Using document from file: {} ({})",
            hex,
            doc_id_file.display()
        ),
    ))
}
