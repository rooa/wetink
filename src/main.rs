mod app;
mod diff;
mod diffview;
mod highlight;
mod layout;
mod render;
mod theme;

use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

const USAGE: &str = "\
wetink — live Markdown viewer that highlights changed blocks

Usage: wetink [OPTIONS] FILE

Options:
  -p, --print        Render once to stdout and exit
  -w, --width N      Maximum content width (default 100)
  -t, --theme T      auto, dark or light (default auto: ask the terminal)
      --mouse        Capture the mouse: click a change to show its diff, wheel to
                     scroll (the terminal's own text selection then needs Shift)
  -h, --help         Show this help
  -V, --version      Show version

Keys: j/k ↑/↓ scroll · space/b page · g/G top/bottom · n/N next/prev change
      d: show diff since opened (or click, with --mouse) · c accept (clear marks) · f toggle follow · q quit
";

const POLL: Duration = Duration::from_millis(100);
const FRAME: Duration = Duration::from_millis(33);

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut print, mut width, mut path, mut mouse) = (false, 100, None, false);
    let mut mode = theme::Mode::Auto;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-p" | "--print" => print = true,
            "--mouse" => mouse = true,
            "-w" | "--width" => match args.next().and_then(|v| v.parse().ok()) {
                Some(w) => width = w,
                None => return fail("--width needs a number"),
            },
            "-t" | "--theme" => match args.next().as_deref() {
                Some("auto") => mode = theme::Mode::Auto,
                Some("dark") => mode = theme::Mode::Dark,
                Some("light") => mode = theme::Mode::Light,
                _ => return fail("--theme needs auto, dark or light"),
            },
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("wetink {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            s if s.starts_with('-') && s.len() > 1 => return fail(&format!("unknown option {s}")),
            _ => path = Some(PathBuf::from(a)),
        }
    }
    let Some(path) = path else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    if path.is_dir() {
        return fail(&format!("{} is a directory", path.display()));
    }
    // Before parsing: styles are fixed when the document is rendered.
    theme::init(mode, io::stdout().is_terminal());
    let result = if print {
        print_once(&path, width)
    } else {
        run(path, width, mouse)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e.to_string()),
    }
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("wetink: {msg}");
    ExitCode::from(2)
}

fn run(path: PathBuf, width: usize, mouse: bool) -> io::Result<()> {
    let mut app = app::App::new(path, width);
    let mut term = ratatui::init();
    if mouse {
        execute!(io::stdout(), EnableMouseCapture)?;
        // ratatui's panic hook restores the screen but not the mouse.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(io::stdout(), DisableMouseCapture);
            hook(info);
        }));
    }
    let result = (|| {
        let mut next_poll = Instant::now() + POLL;
        let mut dirty = true;
        loop {
            let now = Instant::now();
            let flashing = app.flashing(now);
            if dirty || flashing {
                term.draw(|f| app.draw(f, now))?;
                // One more frame after a flash ends clears its last tint.
                dirty = flashing;
            }
            let wait = if flashing {
                FRAME
            } else {
                next_poll.saturating_duration_since(now)
            };
            if event::poll(wait)? {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        if app.key(k) {
                            return Ok(());
                        }
                        dirty = true;
                    }
                    Event::Mouse(m) => match m.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            app.click(m.row);
                            dirty = true;
                        }
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                            app.wheel(m.kind == MouseEventKind::ScrollDown);
                            dirty = true;
                        }
                        _ => {}
                    },
                    Event::Resize(..) => dirty = true,
                    _ => {}
                }
            }
            let now = Instant::now();
            if now >= next_poll {
                dirty |= app.poll(now);
                next_poll = now + POLL;
            }
        }
    })();
    if mouse {
        let _ = execute!(io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    result
}

fn print_once(path: &PathBuf, width: usize) -> io::Result<()> {
    let bytes = std::fs::read(path)?;
    let src = String::from_utf8_lossy(&bytes);
    // Only ask a real terminal: crossterm otherwise falls back to spawning `tput`.
    let width = match io::stdout().is_terminal() {
        true => ratatui::crossterm::terminal::size().map_or(width, |(w, _)| width.min(w as usize)),
        false => width,
    };
    let blocks = render::parse(&src, &mut highlight::Highlighter::default());
    let l = layout::layout(&blocks, &diff::Changes::default(), width);
    let mut out = io::BufWriter::new(io::stdout().lock());
    for row in &l.rows {
        writeln!(out, "{}", ansi(&row.line))?;
    }
    out.flush()
}

fn ansi(line: &Line) -> String {
    let mut s = String::new();
    for span in &line.spans {
        let sgr = sgr(span.style);
        if sgr.is_empty() {
            s.push_str(&span.content);
        } else {
            let _ = write!(s, "\x1b[{sgr}m{}\x1b[0m", span.content);
        }
    }
    s
}

fn sgr(st: Style) -> String {
    let mut p: Vec<String> = Vec::new();
    for (color, base) in [(st.fg, 38), (st.bg, 48)] {
        match color {
            Some(Color::Indexed(n)) => p.push(format!("{base};5;{n}")),
            Some(Color::Rgb(r, g, b)) => p.push(format!("{base};2;{r};{g};{b}")),
            _ => {}
        }
    }
    for (m, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if st.add_modifier.contains(m) {
            p.push(code.into());
        }
    }
    p.join(";")
}
