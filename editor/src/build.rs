use crate::Message;
use fyrox::{
    core::{parking_lot::Mutex, pool::Handle},
    gui::{
        border::BorderBuilder,
        button::{ButtonBuilder, ButtonMessage},
        grid::{Column, GridBuilder, Row},
        message::{MessageDirection, UiMessage},
        scroll_viewer::ScrollViewerBuilder,
        stack_panel::StackPanelBuilder,
        text::{TextBuilder, TextMessage},
        widget::WidgetBuilder,
        window::{WindowBuilder, WindowMessage, WindowTitle},
        BuildContext, Thickness, UiNode, UserInterface, BRUSH_DARKEST,
    },
    gui::{HorizontalAlignment, Orientation},
};
use std::{
    io::{BufRead, BufReader},
    process::ChildStdout,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
};

pub struct BuildWindow {
    window: Handle<UiNode>,
    active: Arc<AtomicBool>,
    changed: Arc<AtomicBool>,
    log: Arc<Mutex<String>>,
    log_text: Handle<UiNode>,
    stop: Handle<UiNode>,
}

impl BuildWindow {
    pub fn new(ctx: &mut BuildContext) -> Self {
        let log_text;
        let stop;
        let window = WindowBuilder::new(WidgetBuilder::new().with_width(300.0).with_height(200.0))
            .can_minimize(false)
            .can_close(false)
            .open(false)
            .with_content(
                GridBuilder::new(
                    WidgetBuilder::new()
                        .with_child(
                            TextBuilder::new(WidgetBuilder::new())
                                .with_text("Please wait while your game is building...\nLog:")
                                .build(ctx),
                        )
                        .with_child(
                            BorderBuilder::new(
                                WidgetBuilder::new()
                                    .on_row(1)
                                    .with_margin(Thickness::uniform(2.0))
                                    .with_background(BRUSH_DARKEST)
                                    .with_child(
                                        ScrollViewerBuilder::new(WidgetBuilder::new())
                                            .with_content({
                                                log_text = TextBuilder::new(WidgetBuilder::new())
                                                    .build(ctx);
                                                log_text
                                            })
                                            .build(ctx),
                                    ),
                            )
                            .build(ctx),
                        )
                        .with_child(
                            StackPanelBuilder::new(
                                WidgetBuilder::new()
                                    .with_horizontal_alignment(HorizontalAlignment::Right)
                                    .on_row(2)
                                    .with_child({
                                        stop = ButtonBuilder::new(
                                            WidgetBuilder::new().with_width(100.0),
                                        )
                                        .with_text("Stop")
                                        .build(ctx);
                                        stop
                                    }),
                            )
                            .with_orientation(Orientation::Horizontal)
                            .build(ctx),
                        ),
                )
                .add_row(Row::auto())
                .add_row(Row::stretch())
                .add_row(Row::strict(28.0))
                .add_column(Column::stretch())
                .build(ctx),
            )
            .with_title(WindowTitle::text("Building the Game..."))
            .build(ctx);

        Self {
            window,
            log_text,
            log: Arc::new(Default::default()),
            active: Arc::new(AtomicBool::new(false)),
            changed: Arc::new(AtomicBool::new(false)),
            stop,
        }
    }

    pub fn listen(&mut self, mut stdout: ChildStdout, ui: &UserInterface) {
        ui.send_message(WindowMessage::open_modal(
            self.window,
            MessageDirection::ToWidget,
            true,
        ));

        let log = self.log.clone();
        self.active.store(true, Ordering::SeqCst);
        let reader_active = self.active.clone();
        let log_changed = self.changed.clone();
        std::thread::spawn(move || {
            while reader_active.load(Ordering::SeqCst) {
                for line in BufReader::new(&mut stdout).lines().take(10).flatten() {
                    log.lock().push_str(&line);
                    log_changed.store(true, Ordering::SeqCst);
                }
            }
        });
    }

    pub fn reset(&mut self, ui: &UserInterface) {
        self.active.store(false, Ordering::SeqCst);
        self.changed.store(false, Ordering::SeqCst);
        self.log.lock().clear();
        ui.send_message(TextMessage::text(
            self.log_text,
            MessageDirection::ToWidget,
            Default::default(),
        ));
        ui.send_message(WindowMessage::close(
            self.window,
            MessageDirection::ToWidget,
        ));
    }

    pub fn update(&mut self, ui: &UserInterface) {
        if self.changed.load(Ordering::SeqCst) {
            ui.send_message(TextMessage::text(
                self.log_text,
                MessageDirection::ToWidget,
                self.log.lock().clone(),
            ));

            self.changed.store(false, Ordering::SeqCst);
        }
    }

    pub fn handle_ui_message(
        &mut self,
        message: &UiMessage,
        sender: &Sender<Message>,
        ui: &UserInterface,
    ) {
        if let Some(ButtonMessage::Click) = message.data() {
            if message.destination() == self.stop {
                sender.send(Message::SwitchToEditMode).unwrap();
                self.reset(ui);
            }
        }
    }
}
