use crossterm::{
    cursor::MoveTo,
    event::{Event, EventStream as CrosstermEventStream, KeyCode, KeyEvent, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use futures::{FutureExt, StreamExt};
use std::{fmt::Write, io::BufWriter, thread::current};
use std::{io::Write as IoWrite, string::FromUtf8Error};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub struct Tui;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TuiMenuOption {
    LoadModel,
    GenerateOutput,
    Exit,
}

impl Into<String> for TuiMenuOption {
    fn into(self) -> String {
        match self {
            Self::LoadModel => String::from("Load model"),
            Self::GenerateOutput => String::from("Generate output from model"),
            Self::Exit => String::from("Close the program"),
        }
    }
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum TuiError {
    #[error("failed to convert vector of u8 to string: {0}")]
    U8ToStringConversionError(#[from] FromUtf8Error),

    #[error("io error occured: {0}")]
    IoError(String),

    #[error("cancel signal has been received")]
    Cancelled,
}

impl From<std::io::Error> for TuiError {
    fn from(value: std::io::Error) -> Self {
        Self::IoError(value.to_string())
    }
}

pub enum TuiKeypress {
    Enter,
    UpArrow,
    DownArrow,
    Escape,
}

impl Tui {
    fn rewrite_menu(current_opt: TuiMenuOption) -> Result<(), TuiError> {
        execute!(
            std::io::stdout(),
            Clear(ClearType::FromCursorUp),
            MoveTo(0, 0)
        )?;

        writeln!(
            std::io::stdout(),
            "Use arrows for navigation, escape for exit and return (enter) for confirmation."
        )?;
        writeln!(std::io::stdout())?;

        let all_opts: Vec<TuiMenuOption> =
            vec![TuiMenuOption::LoadModel, TuiMenuOption::GenerateOutput];

        for opt in all_opts {
            let stringified_opt: String = opt.into();

            let (foreground, background) = if current_opt == opt {
                (Color::Black, Color::White)
            } else {
                (Color::White, Color::Black)
            };

            execute!(
                std::io::stdout(),
                SetForegroundColor(foreground),
                SetBackgroundColor(background),
                Print(stringified_opt),
                ResetColor
            )?;

            writeln!(std::io::stdout(), "")?;
        }

        Ok(())
    }

    pub async fn menu(cancellation_token: CancellationToken) -> Result<TuiMenuOption, TuiError> {
        // this function does not do any long-hanging stuff,
        // so it does not check for token cancellation

        let all_opts: Vec<TuiMenuOption> =
            vec![TuiMenuOption::LoadModel, TuiMenuOption::GenerateOutput];

        let mut current_opt_index = 0;

        Self::rewrite_menu(all_opts[current_opt_index])?;

        loop {
            match Self::listen_for_menu_keys(cancellation_token.clone()).await? {
                TuiKeypress::UpArrow => {
                    if current_opt_index == all_opts.len() - 1 {
                        current_opt_index = 0;
                    }
                }
                TuiKeypress::DownArrow => {
                    if current_opt_index == 0 {
                        current_opt_index = all_opts.len() - 1;
                    }
                }
                TuiKeypress::Enter => return Ok(all_opts[current_opt_index]),
                TuiKeypress::Escape => return Ok(TuiMenuOption::Exit),
            };

            Self::rewrite_menu(all_opts[current_opt_index])?;
        }
    }

    pub async fn listen_for_menu_keys(
        cancellation_token: CancellationToken,
    ) -> Result<TuiKeypress, TuiError> {
        let mut crossterm_ev_stream = CrosstermEventStream::new();

        loop {
            let mut crossterm_ev_stream_future = crossterm_ev_stream.next().fuse();

            tokio::select! {
                () = cancellation_token.cancelled() => {
                    return Err(TuiError::Cancelled);
                },
                value = &mut crossterm_ev_stream_future => {
                    match value {
                        Some(Ok(crossterm::event::Event::Key(kev))) => {
                            if kev.kind == KeyEventKind::Press {
                                match kev.code {
                                    KeyCode::Up => return Ok(TuiKeypress::UpArrow),
                                    KeyCode::Down => return Ok(TuiKeypress::DownArrow),
                                    KeyCode::Esc => return Ok(TuiKeypress::Escape),
                                    KeyCode::Enter => return Ok(TuiKeypress::Enter),
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    };
                },
            };
        }
    }

    pub async fn run(cancellation_token: CancellationToken) -> Result<(), TuiError> {
        let menu_result = Self::menu(cancellation_token.clone()).await?;

        Ok(())
    }
}
