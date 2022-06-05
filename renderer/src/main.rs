use chrono::Local;
use chrono::TimeZone;
use handlebars::Handlebars;
use ignore::DirEntry;
use ignore::WalkBuilder;
use ignore::WalkState::*;
use num_cpus::get;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::fs::create_dir_all;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::Instant;
use html_minifier::HTMLMinifier;

const DISCORD_EPOCH: u64 = 1420070400000;

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct ServerDataRaw {
    Id: String,
    Name: String,
    Icon: String,
    OwnerId: String,
    Description: String,
}

struct ServerData {
    id: u64,
    name: String,
    icon: String,
    owner_id: u64,
    description: String,
}

impl From<ServerDataRaw> for ServerData {
    fn from(s: ServerDataRaw) -> Self {
        Self {
            id: s.Id.parse().unwrap_or(0),
            name: s.Name,
            icon: s.Icon,
            owner_id: s.OwnerId.parse().unwrap_or(0),
            description: s.Description,
        }
    }
}

impl ServerData {
    pub fn register(&self, data: &mut BTreeMap<String, String>) {
        data.insert("server_id".to_string(), self.id.to_string());
        data.insert("server_name".to_string(), self.name.to_owned());
        data.insert("server_icon".to_string(), self.icon.to_owned());
        data.insert("server_owner".to_string(), self.owner_id.to_string());
        data.insert(
            "server_description".to_string(),
            self.description.to_owned(),
        );
    }
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct ChannelDataRaw {
    Id: String,
    Name: String,
    Topic: String,
}

struct ChannelData {
    id: u64,
    name: String,
    topic: String,
}

impl From<ChannelDataRaw> for ChannelData {
    fn from(c: ChannelDataRaw) -> Self {
        Self {
            id: c.Id.parse().unwrap_or(0),
            name: c.Name,
            topic: c.Topic,
        }
    }
}

impl ChannelData {
    pub fn register(&self, data: &mut BTreeMap<String, String>) {
        data.insert("channel_id".to_string(), self.id.to_string());
        data.insert("channel_name".to_string(), self.name.to_owned());
        data.insert("channel_topic".to_string(), self.topic.to_owned());
    }
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Attachments {
    Id: String,
    Url: String,
    Filename: String,
    Size: u32,
    Ephermeral: bool,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Author {
    Username: String,
    Discriminator: String,
    Id: String,
    Mfa: bool,
    Bot: bool,
    Avatar: String,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct ReferencedMessage {
    Id: String,
    Author: Author,
    Attachments: Vec<Attachments>,
    Content: String,
    Pinned: bool,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Message {
    Id: String,
    Author: Author,
    Attachments: Vec<Attachments>,
    Pinned: bool,
    Content: String,
    ReferencedMessage: Vec<ReferencedMessage>,
}

#[inline(always)]
fn format_size(bytes: u32) -> String {
    match bytes {
        1073741824.. => format!("{:.2} GiB", bytes as f64 / 1073741824.0),
        1000000.. => format!("{:.2} MiB", bytes as f64 / 1000000.0),
        1000.. => format!("{:.2} KiB", bytes as f32 / 1000.0),
        _ => format!("{bytes} B"),
    }
}

#[inline(always)]
fn get_date(id: u64) -> String {
    let ms: u64 = id >> 22;
    let dt = Local.timestamp_millis((ms + DISCORD_EPOCH) as i64);
    dt.to_rfc2822()
}

fn main() -> Result<(), Box<std::io::Error>> {
    let startup = Instant::now();
    let core_count = get();
    let _ = fs::create_dir("out");
    let mut handlebars = Handlebars::new();
    let server_template = fs::read_to_string("./index.hbs")?;
    handlebars
        .register_template_string("server", server_template)
        .unwrap();
    for dir in fs::read_dir(".")? {
        let id = match dir?.file_name().to_string_lossy().parse::<u64>() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let server_data_path = format!("{id}/server.json");
        if !PathBuf::from(&server_data_path).exists() {
            println!("does server.json exists for {id}? skipping...");
            continue;
        }
        let server_data: ServerDataRaw =
            match serde_json::from_str(&fs::read_to_string(&server_data_path)?) {
                Ok(v) => v,
                Err(_) => {
                    println!("invalid server data for {server_data_path}, skipping");
                    continue;
                }
            };
        let server_data: ServerData = server_data.into();
        let output_folder = format!("out/{}", server_data.id);
        let _ = create_dir_all(&output_folder);
        let mut channel_map = BTreeMap::new();
        let mut all_channel_data = Vec::new();
        for channel in fs::read_dir(id.to_string())? {
            let channel = channel?;
            let channel_data = PathBuf::from(&format!(
                "{id}/{}/channel.json",
                channel.file_name().to_string_lossy()
            ));
            if !channel_data.exists() {
                continue;
            }
            let channel_data: ChannelDataRaw =
                match serde_json::from_str(&fs::read_to_string(&channel_data)?) {
                    Ok(v) => v,
                    Err(_) => {
                        println!(
                            "invalid channel data for {}",
                            channel.file_name().to_string_lossy()
                        );
                        continue;
                    }
                };
            let channel_data: ChannelData = channel_data.into();
            channel_map.insert(channel_data.id, vec![channel_data.name.clone()]);
            let amount = fs::read_dir(format!("{id}/{}", channel.file_name().to_string_lossy()))?
                .flatten()
                .filter(|x| x.file_name().to_string_lossy().contains("json"))
                .count();
            if amount < 3 {
                continue;
            }
            for i in 0..(amount - 2) {
                channel_map
                    .get_mut(&channel_data.id)
                    .unwrap()
                    .push(format!("{}{}", channel_data.name, i + 1))
            }
            all_channel_data.push(channel_data);
        }
        let mut index_data = BTreeMap::new();
        server_data.register(&mut index_data);
        let mut channels = String::new();
        for (channel_id, channel_names) in channel_map {
            for (i, channel_shard) in channel_names.iter().enumerate() {
                let name = if i == 0 {
                    format!("{channel_id}")
                } else {
                    format!("{channel_id}_{i}")
                };
                channels.push_str(&format!(
                    "<a href=\"./{name}.html\">{channel_shard}</a><br>\n"
                ));
            }
        }
        index_data.insert("channels".to_owned(), channels.to_owned());
        let output = handlebars.render("server", &index_data).unwrap();
        let index_path = format!("{output_folder}/index.html");
        let _ = File::create(&index_path);
        let mut index_output = OpenOptions::new().write(true).open(&index_path)?;
        let _ = index_output.set_len(0);
        let mut html_minifier = HTMLMinifier::new();
        let output = match html_minifier.digest(&output) {
            Ok(_) => html_minifier.get_html(),
            Err(_) => output.as_bytes(),
        };
        index_output.write_all(output)?;
        let (tx, rx) = crossbeam_channel::bounded::<DirEntry>(100);
        let stdout_thread = thread::spawn(move || {
            for dent in rx {
                if let Some(v) = dent.file_type() {
                    if v.is_dir() {
                        continue;
                    }
                }
                if dent.file_name() == "server.json"
                    || dent.file_name() == "channel.json"
                    || !dent.file_name().to_string_lossy().contains("json")
                {
                    continue;
                }
                println!("processing: {:?}...", dent.path());
                let chunk: usize = dent
                    .file_name()
                    .to_string_lossy()
                    .split('.')
                    .collect::<Vec<&str>>()[0]
                    .parse()
                    .unwrap();

                let mut data: Vec<Message> =
                    match serde_json::from_str(&fs::read_to_string(dent.path()).unwrap()) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                data.reverse();

                let mut path = dent.path().iter();
                path.next_back();
                let channel_id = path.next_back();
                let channel_id = channel_id.unwrap().to_string_lossy();
                let mut message_data = String::new();
                for message in data {
                    let reference = if message.ReferencedMessage.is_empty() {
                        String::from("")
                    } else {
                        let msg_attachments = if message.ReferencedMessage[0].Attachments.is_empty()
                        {
                            String::from("")
                        } else {
                            let mut attachment_data = String::new();
                            for attachment in &message.ReferencedMessage[0].Attachments {
                                attachment_data.push_str(&format!(
                                    "
<img src={} alt=\"\" />
<br>
Name: {} Size: {} Id: {} Ephermeral: {}
<br> \n
                                        ",
                                    attachment.Url,
                                    attachment.Filename,
                                    format_size(attachment.Size),
                                    attachment.Id,
                                    attachment.Ephermeral,
                                ));
                            }
                            attachment_data
                        };
                        let mut reply = String::new();
                        for refmsg in &message.ReferencedMessage {
                            reply.push_str(&format!(
                                "
<div class=\"message\">
    <div class=\"author\">
        <img src=\"{}\" alt=\"\" />
        ORIGINAL: {}#{} ({}) <br>
        Bot: {} Mfa: {} Pinned: {} Timestamp: {}
    </div>
    <div class=\"content\">
        {} 
    </div>
    <div class=\"attachments\">
        {}
    </div>
</div>
                                    ",
                                refmsg.Author.Avatar,
                                refmsg.Author.Username,
                                refmsg.Author.Discriminator,
                                refmsg.Author.Id,
                                refmsg.Author.Bot,
                                refmsg.Author.Mfa,
                                refmsg.Pinned,
                                get_date(refmsg.Id.parse().unwrap()),
                                refmsg.Content,
                                msg_attachments,
                            ));
                        }
                        reply
                    };
                    let msg_attachments = if message.Attachments.is_empty() {
                        String::from("")
                    } else {
                        let mut attachment_data = String::new();
                        for attachment in &message.Attachments {
                            attachment_data.push_str(&format!(
                                "
<img src={} alt=\"\" />
<br>
Name: {} Size: {} Id: {} Ephermeral: {}
<br> \n
                                    ",
                                format_args!(
                                    "../../{id}/{channel_id}/{}/{}",
                                    attachment.Id, attachment.Filename
                                ),
                                attachment.Filename,
                                format_size(attachment.Size),
                                attachment.Id,
                                attachment.Ephermeral,
                            ));
                        }
                        attachment_data
                    };
                    let reply_indicator = if reference.is_empty() {
                        String::from("")
                    } else {
                        String::from("REPLY: ")
                    };
                    message_data.push_str(&format!(
                        "
<div class=\"message\">
    <div class=\"reply\">
        {}
    </div>
    <div class=\"author\">
        <img src=\"{}\" alt=\"\" /> <br>
        {reply_indicator}{}#{} ({}) <br>
        Bot: {} Mfa: {} Pinned: {} Timestamp: {}
    </div>
    <div class=\"content\">
        {reply_indicator}{} 
    </div>
    <div class=\"attachments\">
        {}
    </div>
</div>
                            ",
                        reference,
                        message.Author.Avatar,
                        message.Author.Username,
                        message.Author.Discriminator,
                        message.Author.Id,
                        message.Author.Bot,
                        message.Author.Mfa,
                        message.Pinned,
                        get_date(message.Id.parse().unwrap()),
                        message.Content,
                        msg_attachments,
                    ));
                }

                let channel_file = if chunk == 0 {
                    format!("{output_folder}/{channel_id}.html")
                } else {
                    format!("{output_folder}/{channel_id}_{chunk}.html")
                };
                let mut channel_data = BTreeMap::new();
                for channel in &all_channel_data {
                    if channel.id.to_string() == channel_id {
                        channel.register(&mut channel_data);
                    }
                }
                channel_data.insert("channels".to_string(), channels.to_owned());
                channel_data.insert("data".to_string(), message_data);
                channel_data.insert("server_icon".to_string(), server_data.icon.clone());
                channel_data.insert("server_name".to_string(), server_data.name.clone());
                channel_data.insert("server_id".to_string(), server_data.id.to_string());

                let mut handlebars = Handlebars::new();
                let channel_template = fs::read_to_string("./channel.hbs").unwrap();
                handlebars
                    .register_template_string("channel", channel_template)
                    .unwrap();
                let output = handlebars.render("channel", &channel_data).unwrap();
                let mut html_minifier = HTMLMinifier::new();
                let output = match html_minifier.digest(&output) {
                    Ok(_) => html_minifier.get_html(),
                    Err(_) => output.as_bytes(),
                };
                let _ = File::create(&channel_file);
                let mut channel_output =
                    OpenOptions::new().write(true).open(&channel_file).unwrap();
                let _ = channel_output.set_len(0);
                channel_output.write_all(output).unwrap();
            }
        });
        let walker = WalkBuilder::new(id.to_string())
            .threads(core_count)
            .hidden(false)
            .git_ignore(false)
            .build_parallel();
        walker.run(|| {
            let tx = tx.clone();
            Box::new(move |result| {
                if let Ok(v) = result {
                    let _ = tx.send(v);
                }
                Continue
            })
        });
        drop(tx);
        stdout_thread.join().unwrap();
    }
    println!(
        "done in {:#?} using {core_count} threads",
        startup.elapsed()
    );
    Ok(())
}
