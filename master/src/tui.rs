use crossterm::{
    cursor::MoveTo,
    event::{EventStream as CrosstermEventStream, KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use futures::{FutureExt, StreamExt};
use std::{fmt::Display, io::Write as IoWrite, string::FromUtf8Error};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub struct Tui;

pub enum TuiScreen {
    Menu,
    ChooseModel,
    WritePrompt,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TuiMenuOption {
    LoadModel,
    GenerateOutput,
    Exit,
}

impl Display for TuiMenuOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadModel => write!(f, "Load model")?,
            Self::GenerateOutput => write!(f, "Generate output from model")?,
            Self::Exit => write!(f, "Close the program")?,
        };

        Ok(())
    }
}

impl Display for TuiChooseModelMenuOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TuiChooseModelMenuOption::SelectModel(model) => match model {
                ModelType::Llama3v2_1B => write!(f, "{model}")?,
            },
            TuiChooseModelMenuOption::Exit => {
                write!(f, "Do not choose any model, go back to main menu.")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModelType {
    Llama3v2_1B,
}

impl std::fmt::Display for ModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Llama3v2_1B => write!(f, "Llama v3.2 1B")?,
        }

        Ok(())
    }
}

impl Into<proto::master::ModelType> for ModelType {
    fn into(self) -> proto::master::ModelType {
        match self {
            Self::Llama3v2_1B => proto::master::ModelType::Llama3v2_1B,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TuiChooseModelMenuOption {
    SelectModel(ModelType),
    Exit,
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
    fn rewrite_menu(all_opts: &Vec<String>, current_opt: usize) -> Result<(), TuiError> {
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

        for (opt_index, opt_val) in all_opts.iter().enumerate() {
            let (foreground, background) = if current_opt == opt_index {
                (Color::Black, Color::White)
            } else {
                (Color::White, Color::Black)
            };

            execute!(
                std::io::stdout(),
                SetForegroundColor(foreground),
                SetBackgroundColor(background),
                Print(opt_val),
                ResetColor
            )?;

            writeln!(std::io::stdout(), "")?;
        }

        Ok(())
    }

    pub async fn display_option_menu(
        all_opts: &Vec<String>,
        cancellation_token: CancellationToken,
    ) -> Result<Option<usize>, TuiError> {
        // this function does not do any long-hanging stuff,
        // so it does not check for token cancellation.
        let mut current_opt_index = 0;

        Self::rewrite_menu(all_opts, current_opt_index)?;

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
                TuiKeypress::Enter => return Ok(Some(current_opt_index)),
                TuiKeypress::Escape => return Ok(None),
            };

            Self::rewrite_menu(all_opts, current_opt_index)?;
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

    async fn launch_main_menu(
        cancellation_token: CancellationToken,
    ) -> Result<TuiMenuOption, TuiError> {
        let opts = vec![TuiMenuOption::LoadModel, TuiMenuOption::GenerateOutput];

        match Self::display_option_menu(
            &opts.iter().map(|opt| opt.to_string()).collect(),
            cancellation_token,
        )
        .await?
        {
            Some(opt_index) => Ok(opts[opt_index]),
            None => Ok(TuiMenuOption::Exit),
        }
    }

    async fn launch_choose_model_menu(
        cancellation_token: CancellationToken,
    ) -> Result<TuiChooseModelMenuOption, TuiError> {
        let opts = vec![
            TuiChooseModelMenuOption::SelectModel(ModelType::Llama3v2_1B),
            TuiChooseModelMenuOption::Exit,
        ];

        match Self::display_option_menu(
            &opts.iter().map(|opt| opt.to_string()).collect(),
            cancellation_token,
        )
        .await?
        {
            Some(opt_index) => Ok(opts[opt_index]),
            None => Ok(TuiChooseModelMenuOption::Exit),
        }
    }

    pub async fn run(cancellation_token: CancellationToken) -> Result<(), TuiError> {
        let mut tui_screen = TuiScreen::Menu;

        loop {
            match tui_screen {
                TuiScreen::Menu => {
                    match Self::launch_main_menu(cancellation_token.clone()).await? {
                        TuiMenuOption::Exit => return Ok(()),
                        TuiMenuOption::GenerateOutput => tui_screen = TuiScreen::WritePrompt,
                        TuiMenuOption::LoadModel => tui_screen = TuiScreen::ChooseModel,
                    }
                }
                TuiScreen::ChooseModel => {
                    match Self::launch_choose_model_menu(cancellation_token.clone()).await? {
                        TuiChooseModelMenuOption::SelectModel(model) => {
                            panic!("Chosen model: {model}");
                        }
                        TuiChooseModelMenuOption::Exit => tui_screen = TuiScreen::Menu,
                    }
                }
                _ => unimplemented!(),
            }
        }
    }
}
