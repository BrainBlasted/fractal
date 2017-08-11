use std::{self, env, thread};
use std::time::Duration;
use std::sync::{Arc, Mutex};

use futures::{self, Sink};
use gio;
use gtk;
use gtk::prelude::*;
use url::Url;

use bg_thread::{self, Command, ConnectionMethod};

// TODO: Is this the correct format for GApplication IDs?
const APP_ID: &'static str = "jplatte.ruma_gtk";


struct AppLogic {
    gtk_builder: gtk::Builder,

    /// Sender for the matrix channel.
    ///
    /// This channel is used to send commands to the background thread.
    command_chan_tx: futures::sink::Wait<futures::sync::mpsc::Sender<bg_thread::Command>>,
}

impl AppLogic {
    pub fn login(&mut self) {
        let user_entry: gtk::Entry = self.gtk_builder.get_object("login_username").unwrap();
        let pass_entry: gtk::Entry = self.gtk_builder.get_object("login_password").unwrap();
        let server_entry: gtk::Entry = self.gtk_builder.get_object("login_server").unwrap();

        let username = match user_entry.get_text() { Some(s) => s, None => String::from("") };
        let password = match pass_entry.get_text() { Some(s) => s, None => String::from("") };

        println!("Login: {}, {}", username, password);

        self.connect(username, password, server_entry.get_text());
    }

    pub fn connect(&mut self, username: String, password: String, server: Option<String>) {
        let server_url = match server {
            Some(s) => s,
            None => String::from("https://matrix.org")
        };

        let res = self.command_chan_tx
            .send(Command::Connect {
                homeserver_url: Url::parse(&server_url).unwrap(),
                connection_method: ConnectionMethod::Login {
                    username: username,
                    password: password,
                },
            });

        match res {
            Ok(_) => {},
            Err(error) => println!("{:?}", error)
        }
    }
}

/// State for the main thread.
///
/// It takes care of starting up the application and for loading and accessing the
/// UI.
pub struct App {
    /// GTK Application which runs the main loop.
    gtk_app: gtk::Application,

    /// Used to access the UI elements.
    gtk_builder: gtk::Builder,

    /// Channel receiver which allows to run actions from the matrix connection thread.
    ///
    /// Long polling is required to receive messages from the rooms and so they have to
    /// run in separate threads.  In order to allow those threads to modify the gtk content,
    /// they will send closures to the main thread using this channel.
    ui_dispatch_chan_rx: std::sync::mpsc::Receiver<Box<Fn(&gtk::Builder) + Send>>,

    /// Matrix communication thread join handler used to clean up the tread when
    /// closing the application.
    bg_thread_join_handle: thread::JoinHandle<()>,

    logic: Arc<Mutex<AppLogic>>,
}

impl App {
    /// Create an App instance
    pub fn new() -> App {
        let gtk_app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::empty())
            .expect("Failed to initialize GtkApplication");

        let gtk_builder = gtk::Builder::new_from_file("res/main_window.glade");

        let (command_chan_tx, command_chan_rx) = futures::sync::mpsc::channel(1);
        let command_chan_tx = command_chan_tx.wait();

        // Create channel to allow the matrix connection thread to send closures to the main loop.
        let (ui_dispatch_chan_tx, ui_dispatch_chan_rx) = std::sync::mpsc::channel();

        let bg_thread_join_handle =
            thread::spawn(move || bg_thread::run(command_chan_rx, ui_dispatch_chan_tx));

        let logic = Arc::new(Mutex::new(AppLogic{ gtk_builder: gtk_builder.clone(), command_chan_tx }));

        let app = App {
            gtk_app,
            gtk_builder,
            ui_dispatch_chan_rx,
            bg_thread_join_handle,
            logic: logic.clone(),
        };

        app.connect_gtk();
        app
    }

    pub fn connect_gtk(&self) {
        let gtk_builder = self.gtk_builder.clone();
        let logic = self.logic.clone();
        self.gtk_app.connect_activate(move |app| {
            // Set up shutdown callback
            let window: gtk::Window = gtk_builder.get_object("main_window")
                .expect("Couldn't find main_window in ui file.");

            window.connect_delete_event(clone!(app => move |_, _| {
                app.quit();
                Inhibit(false)
            }));

            // Set up user popover
            let user_button: gtk::Button = gtk_builder.get_object("user_button")
                .expect("Couldn't find user_button in ui file.");

            let user_menu: gtk::Popover = gtk_builder.get_object("user_menu")
                .expect("Couldn't find user_menu in ui file.");

            user_button.connect_clicked(move |_| user_menu.show_all());

            // Login click
            let login_btn: gtk::Button = gtk_builder.get_object("login_button")
                .expect("Couldn't find login_button in ui file.");
            let logic_c = logic.clone();
            login_btn.connect_clicked(move |_| logic_c.lock().unwrap().login());

            // Associate window with the Application and show it
            window.set_application(Some(app));
            window.show_all();
        });
    }

    pub fn run(mut self) {
        // Convert the args to a Vec<&str>. Application::run requires argv as &[&str]
        // and envd::args() returns an iterator of Strings.
        let args = env::args().collect::<Vec<_>>();
        let args_refs = args.iter().map(|x| &x[..]).collect::<Vec<_>>();

        // TODO: connect as guess user or use stored data
        //self.logic.lock().unwrap().connect(String::from("TODO"), String::from("TODO"), None);

        // Poll the matrix communication thread channel and run the closures to allow
        // the threads to run actions in the main loop.
        let ui_dispatch_chan_rx = self.ui_dispatch_chan_rx;
        let gtk_builder = self.gtk_builder;
        gtk::idle_add(move || {
            if let Ok(dispatch_fn) = ui_dispatch_chan_rx.recv_timeout(Duration::from_millis(5)) {
                dispatch_fn(&gtk_builder);
            }

            Continue(true)
        });

        // Run the main loop.
        self.gtk_app.run(args_refs.len() as i32, &args_refs);

        // Clean up
        self.bg_thread_join_handle.join().unwrap();
    }
}
