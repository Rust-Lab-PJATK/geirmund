use crossterm::{
    cursor::MoveTo,
    event::{KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::{fmt::Write, io::BufWriter, thread::current};
use std::{io::Write as IoWrite, string::FromUtf8Error};
use thiserror::Error;

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

#[derive(Error, Debug)]
pub enum TuiError {
    #[error("failed to convert vector of u8 to string: {0}")]
    U8ToStringConversionError(#[from] FromUtf8Error),

    #[error("io error occured: {0}")]
    IoError(#[from] std::io::Error),
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

    pub async fn menu() -> Result<TuiMenuOption, TuiError> {
        let all_opts: Vec<TuiMenuOption> =
            vec![TuiMenuOption::LoadModel, TuiMenuOption::GenerateOutput];

        let mut current_opt_index = 0;

        Self::rewrite_menu(all_opts[current_opt_index])?;

        loop {
            match Self::listen_for_menu_keys().await? {
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

    pub async fn listen_for_menu_keys() -> Result<TuiKeypress, TuiError> {
        use crossterm::event::{poll, read, Event};
        use std::time::Duration;

        loop {
            if poll(Duration::from_millis(500))? {
                match read()? {
                    Event::Key(kev) => {
                        if kev.kind != KeyEventKind::Press {
                            continue;
                        }

                        match kev.code {
                            KeyCode::Up => return Ok(TuiKeypress::UpArrow),
                            KeyCode::Down => return Ok(TuiKeypress::DownArrow),
                            KeyCode::Esc => return Ok(TuiKeypress::Escape),
                            KeyCode::Enter => return Ok(TuiKeypress::Enter),
                            _ => {}
                        };
                    }
                    _ => {}
                };
            }
        }
    }

    pub async fn run() -> Result<(), TuiError> {
        let menu = Self::menu().await?;

        Ok(())
    }
}
