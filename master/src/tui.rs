use crossterm::{
    cursor::MoveTo,
    event::{EventStream as CrosstermEventStream, KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use futures::{FutureExt, StreamExt};
use rlpg::tcp::RLPGEventBus;
use std::{
    fmt::Display, future::Future, io::Write as IoWrite, net::SocketAddr, string::FromUtf8Error,
};
use thiserror::Error;
use tokio::{io::AsyncReadExt, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::server::{WorkerGateway, WorkerGatewayCommandError};
use proto::master::ModelType;

enum CurrentFuture {
    LoadModel(JoinHandle<Result<(), WorkerGatewayCommandError>>),
    GenerateResponse(JoinHandle<Result<String, WorkerGatewayCommandError>>),
    None,
}

pub struct Tui {
    connected_client_addr: Option<SocketAddr>,
    rlpg_event_bus: RLPGEventBus,
    tui_screen: TuiScreen,
    current_worker_gateway_bg_task: CurrentFuture,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TuiScreen {
    Menu,
    ChooseModel,
    WritePrompt,
    WaitUntilWorkerLoadsModel,
    WaitUntilWorkerConnects,
    Error(String, Box<TuiScreen>),
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
            TuiChooseModelMenuOption::SelectModel(ModelType::Llama3v2_1B) => {
                write!(f, "Llama v3.2 1B")?
            }
            TuiChooseModelMenuOption::Exit => {
                write!(f, "Do not choose any model, go back to main menu.")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
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

    #[error("tokio broadcast error occured: {0}")]
    TokioReceiveError(#[from] tokio::sync::broadcast::error::RecvError),

    #[error("tokio broadcast error occured: {0}")]
    TokioSendError(String),
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

    async fn display_option_menu(
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

    async fn listen_for_menu_keys(
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
            Some(opt_index) => Ok(opts[opt_index].clone()),
            None => Ok(TuiChooseModelMenuOption::Exit),
        }
    }

    async fn launch_write_prompt_menu(
        &self,
        cancellation_token: CancellationToken,
    ) -> Result<String, TuiError> {
        execute!(
            std::io::stdout(),
            Clear(ClearType::FromCursorUp),
            MoveTo(0, 0),
            Print("Write your prompt: "),
        )?;

        println!();

        let mut buffer = String::new();

        let mut stdin = tokio::io::stdin();

        loop {
            tokio::select! {
                _ = cancellation_token.cancelled() => return Err(TuiError::Cancelled),
                Err(e) = stdin.read_to_string(&mut buffer) => e,
            };

            while buffer.lines().count() > 1 {
                return Ok(buffer);
            }
        }
    }

    pub fn new(rlpg_event_bus: RLPGEventBus) -> Self {
        Self {
            connected_client_addr: None,
            rlpg_event_bus,
            tui_screen: TuiScreen::WaitUntilWorkerConnects,
            current_worker_gateway_bg_task: CurrentFuture::None,
        }
    }

    fn handle_worker_disconnect(&mut self) {
        self.tui_screen = TuiScreen::Error(
            "Worker disconnected".to_string(),
            Box::new(TuiScreen::WaitUntilWorkerConnects),
        );
        self.connected_client_addr = None;
    }

    pub async fn run(&mut self, cancellation_token: CancellationToken) -> Result<(), TuiError> {
        loop {
            match &self.tui_screen {
                TuiScreen::Menu => {
                    let socket_addr = self.connected_client_addr.unwrap();
                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Ok(()),
                        menu_opt = Self::launch_main_menu(cancellation_token.clone())=> match menu_opt {
                            Ok(menu_opt) => match menu_opt {
                                TuiMenuOption::Exit => return Ok(()),
                                TuiMenuOption::GenerateOutput => self.tui_screen = TuiScreen::WritePrompt,
                                TuiMenuOption::LoadModel => self.tui_screen = TuiScreen::ChooseModel,
                            },
                            Err(e) => return Err(e),
                        },
                        _ = WorkerGateway::worker_disconnected(self.rlpg_event_bus.clone(), &socket_addr) => self.handle_worker_disconnect(),
                    };
                }
                TuiScreen::ChooseModel => {
                    let socket_addr = self.connected_client_addr.unwrap();
                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Ok(()),
                        _ = WorkerGateway::worker_disconnected(self.rlpg_event_bus.clone(), &socket_addr) => self.handle_worker_disconnect(),
                        v = Self::launch_choose_model_menu(cancellation_token.clone()) => match v {
                            Ok(TuiChooseModelMenuOption::SelectModel(model)) => {
                                self.current_worker_gateway_bg_task =
                                    CurrentFuture::LoadModel(tokio::spawn(WorkerGateway::load_model(
                                        self.rlpg_event_bus.clone(),
                                        self.connected_client_addr.unwrap(),
                                        model,
                                    )));

                                self.tui_screen = TuiScreen::WaitUntilWorkerLoadsModel;
                            },
                            Ok(TuiChooseModelMenuOption::Exit) => self.tui_screen = TuiScreen::Menu,
                            Err(e) => return Err(e),
                        },
                    };
                }
                TuiScreen::WaitUntilWorkerLoadsModel => {
                    execute!(
                        std::io::stdout(),
                        Print("Waiting for worker to load the model..."),
                        ResetColor,
                    )?;

                    let load_model_task = match self.current_worker_gateway_bg_task {
                        CurrentFuture::LoadModel(ref mut handle) => handle,
                        _ => unreachable!(),
                    };

                    let socket_addr = self.connected_client_addr.unwrap();
                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Ok(()),
                        _ = WorkerGateway::worker_disconnected(self.rlpg_event_bus.clone(), &socket_addr) => self.handle_worker_disconnect(),
                        result = load_model_task => {
                            match result {
                                Ok(_) => self.tui_screen = TuiScreen::WritePrompt,
                                Err(e) => {
                                    let message = format!("Error loading model: {:?}", e);

                                    tracing::error!(message);
                                    self.tui_screen = TuiScreen::Error(message, Box::new(TuiScreen::ChooseModel));
                                }
                            }

                            self.current_worker_gateway_bg_task = CurrentFuture::None;
                        }
                    };
                }
                TuiScreen::WaitUntilWorkerConnects => {
                    execute!(
                        std::io::stdout(),
                        Clear(ClearType::All),
                        MoveTo(0, 0),
                        Print("Waiting for client to connect...")
                    )?;

                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Err(TuiError::Cancelled),
                        addr = WorkerGateway::worker_connected(self.rlpg_event_bus.clone()) => {
                            self.connected_client_addr = Some(addr);
                            self.tui_screen = TuiScreen::Menu;
                        },
                    }
                }
                TuiScreen::WritePrompt => {
                    let socket_addr = self.connected_client_addr.unwrap();
                    tokio::select! {
                        _ = cancellation_token.cancelled() => return Ok(()),
                        _ = WorkerGateway::worker_disconnected(self.rlpg_event_bus.clone(), &socket_addr) => self.handle_worker_disconnect(),
                        prompt = self.launch_write_prompt_menu(cancellation_token.clone()) => {
                            let prompt = prompt?;
                            panic!("{prompt}");
                        }
                    };
                }
                TuiScreen::Error(message, next_screen) => {
                    execute!(
                        std::io::stdout(),
                        Clear(ClearType::All),
                        MoveTo(0, 0),
                        Print(message),
                    )?;

                    println!("");

                    execute!(std::io::stdout(), Print("Press enter to continue."));

                    let mut stdin = tokio::io::stdin();

                    loop {
                        tokio::select! {
                            _ = cancellation_token.cancelled() => return Err(TuiError::Cancelled),
                            Ok(character) = stdin.read_u8() => {
                                if character == '\n' as u8 {
                                    let next_screen: TuiScreen = (&**next_screen).clone(); // idk wtf is going on here, but it compiles
                                    self.tui_screen = next_screen;
                                    break;
                                }
                            },
                        };
                    }
                }
            }
        }
    }
}
