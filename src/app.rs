extern crate glib;
extern crate gtk;
extern crate gio;
extern crate gdk_pixbuf;
extern crate secret_service;
extern crate libnotify;
extern crate chrono;

use self::chrono::prelude::*;

use self::secret_service::SecretService;
use self::secret_service::EncryptionType;

use std::sync::{Arc, Mutex};
use std::sync::mpsc::channel;
use std::sync::mpsc::{Sender, Receiver};
use std::collections::HashMap;
use std::process::Command;

use self::gio::ApplicationExt;
use self::gdk_pixbuf::Pixbuf;
use self::gtk::prelude::*;

use backend::Backend;
use backend::BKCommand;
use backend::BKResponse;
use backend;

use types::Member;
use types::Message;
use types::Protocol;
use types::Room;

use widgets;


#[derive(Debug)]
pub enum Error {
    SecretServiceError,
}

derror!(secret_service::SsError, Error::SecretServiceError);


// TODO: Is this the correct format for GApplication IDs?
const APP_ID: &'static str = "org.gnome.guillotine";


struct TmpMsg {
    pub msg: Message,
    pub widget: gtk::Widget,
}


pub struct AppOp {
    pub gtk_builder: gtk::Builder,
    pub backend: Sender<backend::BKCommand>,
    pub active_room: String,
    pub members: HashMap<String, Member>,
    pub rooms: HashMap<String, Room>,
    pub load_more_btn: gtk::Button,
    pub username: String,
    pub uid: String,
    pub syncing: bool,
    tmp_msgs: Vec<TmpMsg>,
}

#[derive(Debug)]
pub enum MsgPos {
    Top,
    Bottom,
}

#[derive(Debug)]
pub enum RoomPanel {
    Room,
    NoRoom,
    Loading,
}

impl AppOp {
    pub fn login(&self) {
        let user_entry: gtk::Entry = self.gtk_builder
            .get_object("login_username")
            .expect("Can't find login_username in ui file.");
        let pass_entry: gtk::Entry = self.gtk_builder
            .get_object("login_password")
            .expect("Can't find login_password in ui file.");
        let server_entry: gtk::Entry = self.gtk_builder
            .get_object("login_server")
            .expect("Can't find login_server in ui file.");

        let username = match user_entry.get_text() {
            Some(s) => s,
            None => String::from(""),
        };
        let password = match pass_entry.get_text() {
            Some(s) => s,
            None => String::from(""),
        };

        self.connect(username, password, server_entry.get_text());
    }

    pub fn register(&self) {
        let user_entry: gtk::Entry = self.gtk_builder
            .get_object("register_username")
            .expect("Can't find register_username in ui file.");
        let pass_entry: gtk::Entry = self.gtk_builder
            .get_object("register_password")
            .expect("Can't find register_password in ui file.");
        let pass_conf: gtk::Entry = self.gtk_builder
            .get_object("register_password_confirm")
            .expect("Can't find register_password_confirm in ui file.");
        let server_entry: gtk::Entry = self.gtk_builder
            .get_object("register_server")
            .expect("Can't find register_server in ui file.");

        let username = match user_entry.get_text() {
            Some(s) => s,
            None => String::from(""),
        };
        let password = match pass_entry.get_text() {
            Some(s) => s,
            None => String::from(""),
        };
        let passconf = match pass_conf.get_text() {
            Some(s) => s,
            None => String::from(""),
        };

        if password != passconf {
            let window: gtk::Window = self.gtk_builder
                .get_object("main_window")
                .expect("Couldn't find main_window in ui file.");
            let dialog = gtk::MessageDialog::new(Some(&window),
                                                 gtk::DIALOG_MODAL,
                                                 gtk::MessageType::Warning,
                                                 gtk::ButtonsType::Ok,
                                                 "Passwords didn't match, try again");
            dialog.show();

            dialog.connect_response(move |d, _| { d.destroy(); });

            return;
        }

        let server_url = match server_entry.get_text() {
            Some(s) => s,
            None => String::from("https://matrix.org"),
        };

        //self.store_pass(username.clone(), password.clone(), server_url.clone())
        //    .unwrap_or_else(|_| {
        //        // TODO: show an error
        //        println!("Error: Can't store the password using libsecret");
        //    });

        self.show_user_loading();
        let uname = username.clone();
        let pass = password.clone();
        let ser = server_url.clone();
        self.backend.send(BKCommand::Register(uname, pass, ser)).unwrap();
        self.hide_popup();
    }

    pub fn connect(&self, username: String, password: String, server: Option<String>) {
        let server_url = match server {
            Some(s) => s,
            None => String::from("https://matrix.org"),
        };

        self.store_pass(username.clone(), password.clone(), server_url.clone())
            .unwrap_or_else(|_| {
                // TODO: show an error
                println!("Error: Can't store the password using libsecret");
            });

        self.show_user_loading();
        let uname = username.clone();
        let pass = password.clone();
        let ser = server_url.clone();
        self.backend.send(BKCommand::Login(uname, pass, ser)).unwrap();
        self.hide_popup();
    }

    pub fn connect_guest(&self, server: Option<String>) {
        let server_url = match server {
            Some(s) => s,
            None => String::from("https://matrix.org"),
        };

        self.show_user_loading();
        self.backend.send(BKCommand::Guest(server_url)).unwrap();
        self.hide_popup();
    }

    pub fn get_username(&self) {
        self.backend.send(BKCommand::GetUsername).unwrap();
        self.backend.send(BKCommand::GetAvatar).unwrap();
    }

    pub fn set_username(&mut self, username: &str) {
        self.gtk_builder
            .get_object::<gtk::Label>("display_name_label")
            .expect("Can't find display_name_label in ui file.")
            .set_text(username);
        self.show_username();
        self.username = String::from(username);
    }

    pub fn set_uid(&mut self, uid: &str) {
        self.uid = String::from(uid);
    }

    pub fn set_avatar(&self, fname: &str) {
        let image = self.gtk_builder
            .get_object::<gtk::Image>("profile_image")
            .expect("Can't find profile_image in ui file.");

        if let Ok(pixbuf) = Pixbuf::new_from_file_at_size(fname, 20, 20) {
            image.set_from_pixbuf(&pixbuf);
        } else {
            image.set_from_icon_name("image-missing", 2);
        }

        self.show_username();
    }

    pub fn show_username(&self) {
        self.gtk_builder
            .get_object::<gtk::Stack>("user_button_stack")
            .expect("Can't find user_button_stack in ui file.")
            .set_visible_child_name("user_connected_page");
    }

    pub fn show_user_loading(&self) {
        self.gtk_builder
            .get_object::<gtk::Stack>("user_button_stack")
            .expect("Can't find user_button_stack in ui file.")
            .set_visible_child_name("user_loading_page");

        self.room_panel(RoomPanel::Loading);
    }

    pub fn hide_popup(&self) {
        let user_menu: gtk::Popover = self.gtk_builder
            .get_object("user_menu")
            .expect("Couldn't find user_menu in ui file.");
        user_menu.hide();
    }

    pub fn disconnect(&self) {
        self.backend.send(BKCommand::ShutDown).unwrap();
    }

    pub fn store_pass(&self,
                      username: String,
                      password: String,
                      server: String)
                      -> Result<(), Error> {
        let ss = SecretService::new(EncryptionType::Dh)?;
        let collection = ss.get_default_collection()?;

        // deleting previous items
        let allpass = collection.get_all_items()?;
        let passwds = allpass.iter()
            .filter(|x| x.get_label().unwrap_or(String::from("")) == "guillotine");
        for p in passwds {
            p.delete()?;
        }

        // create new item
        collection.create_item(
            "guillotine", // label
            vec![
                ("username", &username),
                ("server", &server),
            ], // properties
            password.as_bytes(), //secret
            true, // replace item with same attributes
            "text/plain" // secret content type
        )?;

        Ok(())
    }

    pub fn get_pass(&self) -> Result<(String, String, String), Error> {
        let ss = SecretService::new(EncryptionType::Dh)?;
        let collection = ss.get_default_collection()?;
        let allpass = collection.get_all_items()?;

        let passwd = allpass.iter()
            .find(|x| x.get_label().unwrap_or(String::from("")) == "guillotine");

        if passwd.is_none() {
            return Err(Error::SecretServiceError);
        }

        let p = passwd.unwrap();
        let attrs = p.get_attributes()?;
        let secret = p.get_secret()?;

        let mut attr = attrs.iter()
            .find(|&ref x| x.0 == "username")
            .ok_or(Error::SecretServiceError)?;
        let username = attr.1.clone();
        attr = attrs.iter()
            .find(|&ref x| x.0 == "server")
            .ok_or(Error::SecretServiceError)?;
        let server = attr.1.clone();

        let tup = (username, String::from_utf8(secret).unwrap(), server);

        Ok(tup)
    }

    pub fn init(&self) {
        if let Ok(pass) = self.get_pass() {
            self.connect(pass.0, pass.1, Some(pass.2));
        } else {
            self.connect_guest(None);
        }
    }

    pub fn room_panel(&self, t: RoomPanel) {
        let s = self.gtk_builder
            .get_object::<gtk::Stack>("room_view_stack")
            .expect("Can't find room_view_stack in ui file.");

        let v = match t {
            RoomPanel::Loading => "loading",
            RoomPanel::Room => "room_view",
            RoomPanel::NoRoom => "noroom",
        };

        s.set_visible_child_name(v);
    }

    pub fn sync(&mut self) {
        if !self.syncing {
            self.syncing = true;
            self.backend.send(BKCommand::Sync).unwrap();
        }
    }

    pub fn set_rooms(&mut self, rooms: Vec<Room>, def: Option<Room>) {
        let store: gtk::TreeStore = self.gtk_builder
            .get_object("rooms_tree_store")
            .expect("Couldn't find rooms_tree_store in ui file.");

        let mut array: Vec<Room> = vec![];

        self.rooms.clear();
        store.clear();

        for r in rooms {
            self.rooms.insert(r.id.clone(), r.clone());
            array.push(r);
        }

        array.sort_by(|x, y| x.name.to_lowercase().cmp(&y.name.to_lowercase()));

        for v in array {
            let ns = match v.notifications {
                0 => String::new(),
                i => format!("{}", i),
            };

            store.insert_with_values(None, None, &[0, 1, 2], &[&v.name, &v.id, &ns]);
        }

        if let Some(d) = def {
            self.set_active_room(d.id, d.name);
        } else {
            self.room_panel(RoomPanel::NoRoom);
        }
    }

    pub fn reload_rooms(&self) {
        self.gtk_builder
            .get_object::<gtk::Stack>("main_content_stack")
            .expect("Can't find main_content_stack in ui file.")
            .set_visible_child_name("Chat");

        self.room_panel(RoomPanel::Loading);
        self.backend.send(BKCommand::SyncForced).unwrap();
    }

    pub fn set_active_room(&mut self, room: String, name: String) {
        self.active_room = room;

        self.room_panel(RoomPanel::Loading);

        let messages = self.gtk_builder
            .get_object::<gtk::ListBox>("message_list")
            .expect("Can't find message_list in ui file.");
        for ch in messages.get_children().iter().skip(1) {
            messages.remove(ch);
        }

        self.members.clear();
        let members = self.gtk_builder
            .get_object::<gtk::ListStore>("members_store")
            .expect("Can't find members_store in ui file.");
        members.clear();

        let name_label = self.gtk_builder
            .get_object::<gtk::Label>("room_name")
            .expect("Can't find room_name in ui file.");
        let edit = self.gtk_builder
            .get_object::<gtk::Entry>("room_name_entry")
            .expect("Can't find room_name_entry in ui file.");
        name_label.set_text(&name);
        edit.set_text(&name);

        // getting room details
        self.backend.send(BKCommand::SetRoom(self.active_room.clone())).unwrap();
    }

    pub fn set_room_detail(&self, key: String, value: String) {
        let k: &str = &key;
        match k {
            "m.room.name" => {
                let name_label = self.gtk_builder
                    .get_object::<gtk::Label>("room_name")
                    .expect("Can't find room_name in ui file.");
                let edit = self.gtk_builder
                    .get_object::<gtk::Entry>("room_name_entry")
                    .expect("Can't find room_name_entry in ui file.");

                name_label.set_text(&value);
                edit.set_text(&value);
            }
            "m.room.topic" => {
                let topic_label = self.gtk_builder
                    .get_object::<gtk::Label>("room_topic")
                    .expect("Can't find room_topic in ui file.");
                let edit = self.gtk_builder
                    .get_object::<gtk::Entry>("room_topic_entry")
                    .expect("Can't find room_topic_entry in ui file.");

                topic_label.set_tooltip_text(&value[..]);
                topic_label.set_text(&value);
                edit.set_text(&value);
            }
            _ => println!("no key {}", key),
        };
    }

    pub fn set_room_avatar(&self, avatar: String) {
        let image = self.gtk_builder
            .get_object::<gtk::Image>("room_image")
            .expect("Can't find room_image in ui file.");
        let config = self.gtk_builder
            .get_object::<gtk::Image>("room_avatar_image")
            .expect("Can't find room_avatar_image in ui file.");

        if !avatar.is_empty() {
            if let Ok(pixbuf) = Pixbuf::new_from_file_at_size(&avatar, 40, 40) {
                image.set_from_pixbuf(&pixbuf);
                config.set_from_pixbuf(&pixbuf);
            }
        } else {
            image.set_from_icon_name("image-missing", 5);
            config.set_from_icon_name("image-missing", 5);
        }
    }

    pub fn scroll_down(&self) {
        let scroll = self.gtk_builder
            .get_object::<gtk::ScrolledWindow>("messages_scroll")
            .expect("Can't find message_scroll in ui file.");

        let s = scroll.clone();
        gtk::timeout_add(500, move || {
            if let Some(adj) = s.get_vadjustment() {
                adj.set_value(adj.get_upper() - adj.get_page_size());
            }
            gtk::Continue(false)
        });
    }

    pub fn add_room_message(&mut self, msg: &Message, msgpos: MsgPos) {
        let messages = self.gtk_builder
            .get_object::<gtk::ListBox>("message_list")
            .expect("Can't find message_list in ui file.");

        if msg.room == self.active_room {
            let m;
            {
                let mb = widgets::MessageBox::new(msg, &self);
                m = mb.widget();
            }

            match msgpos {
                MsgPos::Bottom => messages.add(&m),
                MsgPos::Top => messages.insert(&m, 1),
            };
            self.remove_tmp_room_message(msg);
        } else {
            self.update_room_notifications(&msg.room, |n| n + 1);
        }
    }

    pub fn add_tmp_room_message(&mut self, msg: &Message) {
        let m;
        let messages = self.gtk_builder
            .get_object::<gtk::ListBox>("message_list")
            .expect("Can't find message_list in ui file.");

        {
            let mb = widgets::MessageBox::new(msg, &self);
            m = mb.widget();
        }

        messages.add(&m);
        if let Some(w) = messages.get_children().iter().last() {
            self.tmp_msgs.push(TmpMsg {
                    msg: msg.clone(),
                    widget: w.clone(),
            });

            self.scroll_down();
        };
    }

    pub fn remove_tmp_room_message(&mut self, msg: &Message) {
        let messages = self.gtk_builder
            .get_object::<gtk::ListBox>("message_list")
            .expect("Can't find message_list in ui file.");

        let mut rmidxs = vec![];

        for (i, t) in self.tmp_msgs.iter().enumerate() {
            if t.msg.sender == msg.sender &&
               t.msg.mtype == msg.mtype &&
               t.msg.room == msg.room &&
               t.msg.body == msg.body {

                messages.remove(&t.widget);
                //t.widget.destroy();
                rmidxs.push(i);
            }
        }

        for i in rmidxs {
            self.tmp_msgs.remove(i);
        }
    }

    pub fn update_room_notifications(&self, roomid: &str, f: fn(i32) -> i32) {
        let store: gtk::TreeStore = self.gtk_builder
            .get_object("rooms_tree_store")
            .expect("Couldn't find rooms_tree_store in ui file.");

        if let Some(iter) = store.get_iter_first() {
            loop {
                let v1 = store.get_value(&iter, 1);
                let id: &str = v1.get().unwrap();
                let v2 = store.get_value(&iter, 2);
                let ns: &str = v2.get().unwrap();
                let res: Result<i32, _> = ns.parse();
                let n: i32 = f(res.unwrap_or(0));
                let formatted = match n {
                    0 => String::from(""),
                    i => format!("{}", i),
                };
                if id == roomid {
                    store.set_value(&iter, 2, &gtk::Value::from(&formatted));
                }
                if !store.iter_next(&iter) {
                    break;
                }
            }
        }
    }

    pub fn mark_as_read(&self, msg: &Message) {
        self.backend.send(BKCommand::MarkAsRead(msg.room.clone(),
                                                msg.id.clone())).unwrap();
    }

    pub fn add_room_member(&mut self, m: Member) {
        let store: gtk::ListStore = self.gtk_builder
            .get_object("members_store")
            .expect("Couldn't find members_store in ui file.");

        let name = m.get_alias();

        store.insert_with_values(None, &[0, 1], &[&name, &(m.uid)]);

        self.members.insert(m.uid.clone(), m);
    }

    pub fn member_clicked(&self, uid: String) {
        println!("member clicked: {}, {:?}", uid, self.members.get(&uid));
    }

    pub fn send_message(&mut self, msg: String) {
        let room = self.active_room.clone();
        let now = Local::now();

        let m = Message {
            sender: self.uid.clone(),
            mtype: strn!("m.text"),
            body: msg.clone(),
            room: room.clone(),
            date: now,
            thumb: String::from(""),
            url: String::from(""),
            id: String::from(""),
        };

        self.add_tmp_room_message(&m);
        self.backend.send(BKCommand::SendMsg(m)).unwrap();
    }

    pub fn attach_file(&mut self) {
        let window: gtk::ApplicationWindow = self.gtk_builder
            .get_object("main_window")
            .expect("Can't find main_window in ui file.");
        let dialog = gtk::FileChooserDialog::new(None,
                                                 Some(&window),
                                                 gtk::FileChooserAction::Open);

        let btn = dialog.add_button("Select", 1);
        btn.get_style_context().unwrap().add_class("suggested-action");

        let backend = self.backend.clone();
        let room = self.active_room.clone();
        dialog.connect_response(move |dialog, resp| {
            if resp == 1 {
                if let Some(fname) = dialog.get_filename() {
                    let f = strn!(fname.to_str().unwrap_or(""));
                    backend.send(BKCommand::AttachFile(room.clone(), f)).unwrap();
                }
            }
            dialog.destroy();
        });

        let backend = self.backend.clone();
        let room = self.active_room.clone();
        dialog.connect_file_activated(move |dialog| {
            if let Some(fname) = dialog.get_filename() {
                let f = strn!(fname.to_str().unwrap_or(""));
                backend.send(BKCommand::AttachFile(room.clone(), f)).unwrap();
            }
            dialog.destroy();
        });

        dialog.show();
    }

    pub fn hide_members(&self) {
        self.gtk_builder
            .get_object::<gtk::Stack>("sidebar_stack")
            .expect("Can't find sidebar_stack in ui file.")
            .set_visible_child_name("sidebar_hidden");
    }

    pub fn show_members(&self) {
        self.gtk_builder
            .get_object::<gtk::Stack>("sidebar_stack")
            .expect("Can't find sidebar_stack in ui file.")
            .set_visible_child_name("sidebar_members");
    }

    pub fn load_more_messages(&self) {
        let room = self.active_room.clone();
        self.load_more_btn.set_label("loading...");
        self.backend.send(BKCommand::GetRoomMessagesTo(room)).unwrap();
    }

    pub fn load_more_normal(&self) {
        self.load_more_btn.set_label("load more messages");
    }

    pub fn init_protocols(&self) {
        self.backend.send(BKCommand::DirectoryProtocols).unwrap();
    }

    pub fn set_protocols(&self, protocols: Vec<Protocol>) {
        let combo = self.gtk_builder
            .get_object::<gtk::ListStore>("protocol_model")
            .expect("Can't find protocol_model in ui file.");
        combo.clear();

        for p in protocols {
            combo.insert_with_values(None, &[0, 1], &[&p.desc, &p.id]);
        }

        self.gtk_builder
            .get_object::<gtk::ComboBox>("directory_combo")
            .expect("Can't find directory_combo in ui file.")
            .set_active(0);
    }

    pub fn search_rooms(&self, more: bool) {
        let combo_store = self.gtk_builder
            .get_object::<gtk::ListStore>("protocol_model")
            .expect("Can't find protocol_model in ui file.");
        let combo = self.gtk_builder
            .get_object::<gtk::ComboBox>("directory_combo")
            .expect("Can't find directory_combo in ui file.");

        let active = combo.get_active();
        let protocol: String = match combo_store.iter_nth_child(None, active) {
            Some(it) => {
                let v = combo_store.get_value(&it, 1);
                v.get().unwrap()
            }
            None => String::from(""),
        };

        let q = self.gtk_builder
            .get_object::<gtk::Entry>("directory_search_entry")
            .expect("Can't find directory_search_entry in ui file.");

        let btn = self.gtk_builder
            .get_object::<gtk::Button>("directory_search_button")
            .expect("Can't find directory_search_button in ui file.");
        btn.set_label("Searching...");
        btn.set_sensitive(false);

        if !more {
            let directory = self.gtk_builder
                .get_object::<gtk::ListBox>("directory_room_list")
                .expect("Can't find directory_room_list in ui file.");
            for ch in directory.get_children() {
                directory.remove(&ch);
            }
        }

        self.backend
            .send(BKCommand::DirectorySearch(q.get_text().unwrap(), protocol, more))
            .unwrap();
    }

    pub fn load_more_rooms(&self) {
        self.search_rooms(true);
    }

    pub fn set_directory_room(&self, room: Room) {
        let directory = self.gtk_builder
            .get_object::<gtk::ListBox>("directory_room_list")
            .expect("Can't find directory_room_list in ui file.");

        let rb = widgets::RoomBox::new(&room, &self);
        let room_widget = rb.widget();
        directory.add(&room_widget);

        let btn = self.gtk_builder
            .get_object::<gtk::Button>("directory_search_button")
            .expect("Can't find directory_search_button in ui file.");
        btn.set_label("Search");
        btn.set_sensitive(true);
    }

    pub fn notify(&self, msg: &Message) {
        let roomname = match self.rooms.get(&msg.room) {
            Some(r) => r.name.clone(),
            None => msg.room.clone(),
        };

        let mut body = msg.body.clone();
        body.truncate(80);

        let (tx, rx): (Sender<(String, String)>, Receiver<(String, String)>) = channel();
        self.backend.send(BKCommand::GetUserInfoAsync(msg.sender.clone(), tx)).unwrap();
        gtk::timeout_add(50, move || match rx.try_recv() {
            Err(_) => gtk::Continue(true),
            Ok((name, avatar)) => {
                let summary = format!("@{} / {}", name, roomname);
                let n = libnotify::Notification::new(&summary, Some(&body[..]), Some(&avatar[..]));
                n.show().unwrap();
                gtk::Continue(false)
            }
        });
    }

    pub fn show_room_messages(&mut self, msgs: Vec<Message>, init: bool) {
        for msg in msgs.iter() {
            self.add_room_message(msg, MsgPos::Bottom);

            let mut should_notify = true;
            // not notifying the initial messages
            should_notify = should_notify && !init;
            // not notifying my own messages
            should_notify = should_notify && (msg.sender != self.uid);

            if should_notify {
                self.notify(msg);
            }
        }

        if !msgs.is_empty() {
            let fs = msgs.iter().filter(|x| x.room == self.active_room);
            if let Some(msg) = fs.last() {
                self.scroll_down();
                self.mark_as_read(msg);
            }
        }

        if init {
            self.room_panel(RoomPanel::Room);
        }
    }

    pub fn show_room_dialog(&self) {
        let dialog = self.gtk_builder
            .get_object::<gtk::Dialog>("room_config_dialog")
            .expect("Can't find room_config_dialog in ui file.");

        dialog.show();
    }

    pub fn leave_active_room(&mut self) {
        let r = self.active_room.clone();
        self.backend.send(BKCommand::LeaveRoom(r.clone())).unwrap();
        self.rooms.remove(&r);
        self.active_room = String::new();
        self.room_panel(RoomPanel::NoRoom);

        let store: gtk::TreeStore = self.gtk_builder
            .get_object("rooms_tree_store")
            .expect("Couldn't find rooms_tree_store in ui file.");

        if let Some(iter) = store.get_iter_first() {
            loop {
                let v1 = store.get_value(&iter, 1);
                let id: &str = v1.get().unwrap();
                if id == r {
                    store.remove(&iter);
                }
                if !store.iter_next(&iter) {
                    break;
                }
            }
        }
    }

    pub fn change_room_config(&mut self) {
        let name = self.gtk_builder
            .get_object::<gtk::Entry>("room_name_entry")
            .expect("Can't find room_name_entry in ui file.");
        let topic = self.gtk_builder
            .get_object::<gtk::Entry>("room_topic_entry")
            .expect("Can't find room_topic_entry in ui file.");
        let avatar_fs = self.gtk_builder
            .get_object::<gtk::FileChooserButton>("room_avatar_filechooser")
            .expect("Can't find room_avatar_filechooser in ui file.");

        if let Some(r) = self.rooms.get(&self.active_room) {
            if let Some(n) = name.get_text() {
                if n != r.name {
                    let command = BKCommand::SetRoomName(r.id.clone(), n.clone());
                    self.backend.send(command).unwrap();
                }
            }
            if let Some(t) = topic.get_text() {
                if t != r.topic {
                    let command = BKCommand::SetRoomTopic(r.id.clone(), t.clone());
                    self.backend.send(command).unwrap();
                }
            }
            if let Some(f) = avatar_fs.get_filename() {
                if let Some(name) = f.to_str() {
                    let command = BKCommand::SetRoomAvatar(r.id.clone(), String::from(name));
                    self.backend.send(command).unwrap();
                }
            }
        }
    }

    pub fn room_name_change(&mut self, roomid: String, name: String) {
        let store: gtk::TreeStore = self.gtk_builder
            .get_object("rooms_tree_store")
            .expect("Couldn't find rooms_tree_store in ui file.");

        let mut r = self.rooms.get_mut(&roomid).unwrap();
        r.name = name.clone();

        if roomid == self.active_room {
            self.gtk_builder
                .get_object::<gtk::Label>("room_name")
                .expect("Can't find room_name in ui file.")
                .set_text(&name);
        }

        if let Some(iter) = store.get_iter_first() {
            loop {
                let v1 = store.get_value(&iter, 1);
                let id: &str = v1.get().unwrap();
                if id == roomid {
                    store.set_value(&iter, 0, &gtk::Value::from(&name));
                }
                if !store.iter_next(&iter) {
                    break;
                }
            }
        }
    }

    pub fn room_topic_change(&mut self, roomid: String, topic: String) {
        let mut r = self.rooms.get_mut(&roomid).unwrap();
        r.topic = topic.clone();

        if roomid == self.active_room {
            let t = self.gtk_builder
                .get_object::<gtk::Label>("room_topic")
                .expect("Can't find room_topic in ui file.");

            t.set_tooltip_text(&topic[..]);
            t.set_text(&topic);
        }
    }

    pub fn new_room_avatar(&self, roomid: String) {
        self.backend.send(BKCommand::GetRoomAvatar(roomid)).unwrap();
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

    op: Arc<Mutex<AppOp>>,
}

impl App {
    /// Create an App instance
    pub fn new() -> App {
        let gtk_app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::empty())
            .expect("Failed to initialize GtkApplication");

        let (tx, rx): (Sender<BKResponse>, Receiver<BKResponse>) = channel();

        let bk = Backend::new(tx);
        let apptx = bk.run();

        let gtk_builder = gtk::Builder::new_from_file("res/main_window.glade");
        let op = Arc::new(Mutex::new(AppOp {
            gtk_builder: gtk_builder.clone(),
            load_more_btn: gtk::Button::new_with_label("Load more messages"),
            backend: apptx,
            active_room: String::from(""),
            members: HashMap::new(),
            rooms: HashMap::new(),
            username: String::new(),
            uid: String::new(),
            syncing: false,
            tmp_msgs: vec![],
        }));

        // Sync loop every 3 seconds
        let syncop = op.clone();
        gtk::timeout_add(3000, move || {
            syncop.lock().unwrap().sync();
            gtk::Continue(true)
        });

        let theop = op.clone();
        gtk::timeout_add(500, move || {
            let recv = rx.try_recv();
            match recv {
                Ok(BKResponse::Token(uid, _)) => {
                    theop.lock().unwrap().set_uid(&uid);
                    theop.lock().unwrap().set_username(&uid);
                    theop.lock().unwrap().get_username();
                    theop.lock().unwrap().sync();

                    theop.lock().unwrap().init_protocols();
                }
                Ok(BKResponse::Name(username)) => {
                    theop.lock().unwrap().set_username(&username);
                }
                Ok(BKResponse::Avatar(path)) => {
                    theop.lock().unwrap().set_avatar(&path);
                }
                Ok(BKResponse::Sync) => {
                    println!("SYNC");
                    theop.lock().unwrap().syncing = false;
                }
                Ok(BKResponse::Rooms(rooms, default)) => {
                    theop.lock().unwrap().set_rooms(rooms, default);
                }
                Ok(BKResponse::RoomDetail(key, value)) => {
                    theop.lock().unwrap().set_room_detail(key, value);
                }
                Ok(BKResponse::RoomAvatar(avatar)) => {
                    theop.lock().unwrap().set_room_avatar(avatar);
                }
                Ok(BKResponse::RoomMessages(msgs)) => {
                    theop.lock().unwrap().show_room_messages(msgs, false);
                }
                Ok(BKResponse::RoomMessagesInit(msgs)) => {
                    theop.lock().unwrap().show_room_messages(msgs, true);
                }
                Ok(BKResponse::RoomMessagesTo(msgs)) => {
                    for msg in msgs.iter().rev() {
                        theop.lock().unwrap().add_room_message(msg, MsgPos::Top);
                    }
                    theop.lock().unwrap().load_more_normal();
                }
                Ok(BKResponse::RoomMembers(members)) => {
                    let mut ms = members;
                    ms.sort_by(|x, y| {
                        x.get_alias().to_lowercase().cmp(&y.get_alias().to_lowercase())
                    });
                    for m in ms {
                        theop.lock().unwrap().add_room_member(m);
                    }
                }
                Ok(BKResponse::SendMsg) => {
                    theop.lock().unwrap().sync();
                }
                Ok(BKResponse::DirectoryProtocols(protocols)) => {
                    theop.lock().unwrap().set_protocols(protocols);
                }
                Ok(BKResponse::DirectorySearch(rooms)) => {
                    for room in rooms {
                        theop.lock().unwrap().set_directory_room(room);
                    }
                }
                Ok(BKResponse::JoinRoom) => {
                    theop.lock().unwrap().reload_rooms();
                }
                Ok(BKResponse::LeaveRoom) => { }
                Ok(BKResponse::SetRoomName) => { }
                Ok(BKResponse::SetRoomTopic) => { }
                Ok(BKResponse::SetRoomAvatar) => { }
                Ok(BKResponse::MarkedAsRead(r, _)) => {
                    theop.lock().unwrap().update_room_notifications(&r, |_| 0);
                }

                Ok(BKResponse::RoomName(roomid, name)) => {
                    theop.lock().unwrap().room_name_change(roomid, name);
                }
                Ok(BKResponse::RoomTopic(roomid, topic)) => {
                    theop.lock().unwrap().room_topic_change(roomid, topic);
                }
                Ok(BKResponse::NewRoomAvatar(roomid)) => {
                    theop.lock().unwrap().new_room_avatar(roomid);
                }
                Ok(BKResponse::Media(fname)) => {
                    Command::new("xdg-open")
                                .arg(&fname)
                                .spawn()
                                .expect("failed to execute process");
                }
                Ok(BKResponse::AttachedFile(msg)) => {
                    theop.lock().unwrap().add_tmp_room_message(&msg);
                }

                // errors
                Ok(BKResponse::SyncError(_)) => {
                    println!("SYNC Error");
                    theop.lock().unwrap().syncing = false;
                }
                Ok(err) => {
                    println!("Query error: {:?}", err);
                }
                Err(_) => {}
            };

            gtk::Continue(true)
        });

        let app = App {
            gtk_app: gtk_app,
            gtk_builder: gtk_builder,
            op: op.clone(),
        };

        app.connect_gtk();

        app
    }

    pub fn connect_gtk(&self) {
        // Set up shutdown callback
        let window: gtk::Window = self.gtk_builder
            .get_object("main_window")
            .expect("Couldn't find main_window in ui file.");

        window.set_title("Guillotine");
        let _ = window.set_icon_from_file("res/icon.svg");
        window.show_all();

        let op = self.op.clone();
        window.connect_delete_event(move |_, _| {
            op.lock().unwrap().disconnect();
            gtk::main_quit();
            Inhibit(false)
        });

        self.gtk_app.connect_startup(move |app| { window.set_application(app); });

        self.create_load_more_btn();

        self.connect_user_button();
        self.connect_login_button();
        self.connect_register_button();
        self.connect_guest_button();

        self.connect_room_treeview();
        self.connect_member_treeview();

        self.connect_msg_scroll();

        self.connect_send();
        self.connect_attach();

        self.connect_directory();
        self.connect_room_config();
    }

    fn connect_room_config(&self) {
        // room config button
        let mut btn = self.gtk_builder
            .get_object::<gtk::Button>("room_config_button")
            .expect("Can't find room_config_button in ui file.");
        let mut op = self.op.clone();
        btn.connect_clicked(move |_| {
            op.lock().unwrap().show_room_dialog();
        });

        // room leave button
        let dialog = self.gtk_builder
            .get_object::<gtk::Dialog>("room_config_dialog")
            .expect("Can't find room_config_dialog in ui file.");
        btn = self.gtk_builder
            .get_object::<gtk::Button>("room_leave_button")
            .expect("Can't find room_leave_button in ui file.");
        op = self.op.clone();
        let d = dialog.clone();
        btn.connect_clicked(move |_| {
            op.lock().unwrap().leave_active_room();
            d.hide();
        });

        btn = self.gtk_builder
            .get_object::<gtk::Button>("room_dialog_close")
            .expect("Can't find room_dialog_close in ui file.");
        let d = dialog.clone();
        btn.connect_clicked(move |_| {
            d.hide();
        });

        // TODO: connect OK
        let avatar = self.gtk_builder
            .get_object::<gtk::Image>("room_avatar_image")
            .expect("Can't find room_avatar_image in ui file.");
        let avatar_fs = self.gtk_builder
            .get_object::<gtk::FileChooserButton>("room_avatar_filechooser")
            .expect("Can't find room_avatar_filechooser in ui file.");
        avatar_fs.connect_selection_changed(move |fs| {
            if let Some(fname) = fs.get_filename() {
                if let Some(name) = fname.to_str() {
                    if let Ok(pixbuf) = Pixbuf::new_from_file_at_size(name, 40, 40) {
                        avatar.set_from_pixbuf(&pixbuf);
                    } else {
                        avatar.set_from_icon_name("image-missing", 5);
                    }
                }
            }
        });

        btn = self.gtk_builder
            .get_object::<gtk::Button>("room_dialog_set")
            .expect("Can't find room_dialog_set in ui file.");
        let d = dialog.clone();
        op = self.op.clone();
        btn.connect_clicked(move |_| {
            op.lock().unwrap().change_room_config();
            d.hide();
        });
    }

    fn connect_directory(&self) {
        let btn = self.gtk_builder
            .get_object::<gtk::Button>("directory_search_button")
            .expect("Can't find directory_search_button in ui file.");
        let q = self.gtk_builder
            .get_object::<gtk::Entry>("directory_search_entry")
            .expect("Can't find directory_search_entry in ui file.");

        let scroll = self.gtk_builder
            .get_object::<gtk::ScrolledWindow>("directory_scroll")
            .expect("Can't find directory_scroll in ui file.");

        let mut op = self.op.clone();
        btn.connect_clicked(move |_| { op.lock().unwrap().search_rooms(false); });

        op = self.op.clone();
        scroll.connect_edge_reached(move |_, dir| if dir == gtk::PositionType::Bottom {
            op.lock().unwrap().load_more_rooms();
        });

        op = self.op.clone();
        q.connect_activate(move |_| { op.lock().unwrap().search_rooms(false); });
    }

    fn create_load_more_btn(&self) {
        let messages = self.gtk_builder
            .get_object::<gtk::ListBox>("message_list")
            .expect("Can't find message_list in ui file.");

        let btn = self.op.lock().unwrap().load_more_btn.clone();
        btn.show();
        messages.add(&btn);

        let op = self.op.clone();
        btn.connect_clicked(move |_| { op.lock().unwrap().load_more_messages(); });
    }

    fn connect_msg_scroll(&self) {
        let s = self.gtk_builder
            .get_object::<gtk::ScrolledWindow>("messages_scroll")
            .expect("Can't find message_scroll in ui file.");

        let op = self.op.clone();
        s.connect_edge_overshot(move |_, dir| if dir == gtk::PositionType::Top {
            op.lock().unwrap().load_more_messages();
        });
    }

    fn connect_send(&self) {
        let send_button: gtk::ToolButton = self.gtk_builder
            .get_object("send_button")
            .expect("Couldn't find send_button in ui file.");
        let msg_entry: gtk::Entry = self.gtk_builder
            .get_object("msg_entry")
            .expect("Couldn't find msg_entry in ui file.");

        let entry = msg_entry.clone();
        let mut op = self.op.clone();
        send_button.connect_clicked(move |_| if let Some(text) = entry.get_text() {
            op.lock().unwrap().send_message(text);
            entry.set_text("");
        });

        op = self.op.clone();
        msg_entry.connect_activate(move |entry| if let Some(text) = entry.get_text() {
            op.lock().unwrap().send_message(text);
            entry.set_text("");
        });
    }

    fn connect_attach(&self) {
        let attach_button: gtk::ToolButton = self.gtk_builder
            .get_object("attach_button")
            .expect("Couldn't find attach_button in ui file.");

        let op = self.op.clone();
        attach_button.connect_clicked(move |_| {
            op.lock().unwrap().attach_file();
        });
    }

    fn connect_user_button(&self) {
        // Set up user popover
        let user_button: gtk::Button = self.gtk_builder
            .get_object("user_button")
            .expect("Couldn't find user_button in ui file.");

        let user_menu: gtk::Popover = self.gtk_builder
            .get_object("user_menu")
            .expect("Couldn't find user_menu in ui file.");

        user_button.connect_clicked(move |_| user_menu.show_all());
    }

    fn connect_login_button(&self) {
        // Login click
        let login_btn: gtk::Button = self.gtk_builder
            .get_object("login_button")
            .expect("Couldn't find login_button in ui file.");

        let op = self.op.clone();
        login_btn.connect_clicked(move |_| op.lock().unwrap().login());
    }

    fn connect_register_button(&self) {
        let btn: gtk::Button = self.gtk_builder
            .get_object("register_button")
            .expect("Couldn't find register_button in ui file.");

        let op = self.op.clone();
        btn.connect_clicked(move |_| op.lock().unwrap().register());
    }

    fn connect_guest_button(&self) {
        let btn: gtk::Button = self.gtk_builder
            .get_object("guest_button")
            .expect("Couldn't find guest_button in ui file.");

        let op = self.op.clone();
        let builder = self.gtk_builder.clone();
        btn.connect_clicked(move |_| {
            let server: gtk::Entry = builder.get_object("guest_server")
                .expect("Can't find guest_server in ui file.");
            op.lock().unwrap().connect_guest(server.get_text());
        });
    }

    fn connect_room_treeview(&self) {
        // room selection
        let treeview: gtk::TreeView = self.gtk_builder
            .get_object("rooms_tree_view")
            .expect("Couldn't find rooms_tree_view in ui file.");

        let op = self.op.clone();
        treeview.set_activate_on_single_click(true);
        treeview.connect_row_activated(move |view, path, _| {
            let iter = view.get_model().unwrap().get_iter(path).unwrap();
            let id = view.get_model().unwrap().get_value(&iter, 1);
            let name = view.get_model().unwrap().get_value(&iter, 0);
            op.lock().unwrap().set_active_room(id.get().unwrap(), name.get().unwrap());
        });
    }

    fn connect_member_treeview(&self) {
        // member selection
        let members: gtk::TreeView = self.gtk_builder
            .get_object("members_treeview")
            .expect("Couldn't find members_treeview in ui file.");

        let op = self.op.clone();
        members.set_activate_on_single_click(true);
        members.connect_row_activated(move |view, path, _| {
            let iter = view.get_model().unwrap().get_iter(path).unwrap();
            let id = view.get_model().unwrap().get_value(&iter, 1);
            op.lock().unwrap().member_clicked(id.get().unwrap());
        });

        let mbutton: gtk::Button = self.gtk_builder
            .get_object("members_hide_button")
            .expect("Couldn't find members_hide_button in ui file.");
        let mbutton2: gtk::Button = self.gtk_builder
            .get_object("members_show_button")
            .expect("Couldn't find members_show_button in ui file.");

        let op = self.op.clone();
        mbutton.connect_clicked(move |_| { op.lock().unwrap().hide_members(); });
        let op = self.op.clone();
        mbutton2.connect_clicked(move |_| { op.lock().unwrap().show_members(); });

    }

    pub fn run(self) {
        self.op.lock().unwrap().init();

        if let Err(err) = libnotify::init("guillotine") {
            println!("Error: can't init notifications: {}", err);
        };

        glib::set_application_name("guillotine");
        glib::set_prgname(Some("guillotine"));

        gtk::main();

        libnotify::uninit();
    }
}
