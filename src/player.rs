use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};

use actix::{Actor, ActorContext, Addr, AsyncContext, Handler, Message, StreamHandler};
use actix_web::{
    get,
    web::{Data, Payload, Query},
    HttpRequest, HttpResponse, Result,
};
use actix_web_actors::ws;
use askama::Template;
use rand::seq::SliceRandom;
use serde::{de::Visitor, Deserialize, Serialize};

use crate::{
    session::{self, SessionManager},
    Config, Html, Shitpost,
};

mod templates {
    use askama::Template;

    use crate::Shitpost;

    #[derive(Template)]
    #[template(path = "player.html")]
    pub struct Player<'a> {
        pub shitposts: &'a [Shitpost],
        pub session: &'a str,
    }

    #[derive(Template)]
    #[template(path = "host.html")]
    pub struct Host<'a> {
        pub folders: &'a [&'a str],
        pub session: &'a str,
    }

    #[derive(Template)]
    #[template(path = "join.html")]
    pub struct Join;

    #[derive(Template)]
    #[template(path = "index.html")]
    pub struct Index;

    #[derive(Template)]
    #[template(path = "error.html")]
    pub struct Error<'a> {
        pub text: &'a str,
    }
}
const VALID_FILETYPES: &[&str] = &["mp4", "MP4", "webm"];

#[derive(Deserialize)]
struct SessionConfig {
    amount: usize,
    session: String,
}

struct RouletteFolders(Vec<String>);

impl<'de> Deserialize<'de> for RouletteFolders {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = RouletteFolders;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("folders")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let mut folders = Vec::new();

                while let Some(key) = map.next_key::<&str>()? {
                    if key == "folders" {
                        folders.push(map.next_value::<String>()?);
                    }
                }
                Ok(RouletteFolders(folders))
            }
        }
        deserializer.deserialize_identifier(FieldVisitor)
    }
}

/// The messages sent from the player site itself
#[derive(Deserialize, Serialize)]
enum PlayerMessage {
    Seeked,
    StateChanged(State),
    Position(f64),
    PlaylistChanged(usize),
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum BackendMessage {
    SyncPosition,
    ChangeState(State),
    ChangePosition(f64),
    ChangePlaylist(usize),
}

#[derive(Deserialize)]
struct SessionQuery {
    session: String,
}

/// OvenPlayer state
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Idle,
    Complete,
    Paused,
    Playing,
    Error,
    Loading,
    Stalled,
    AdLoaded,
    AdPlaying,
    AdPaused,
    AdComplete,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct ChangeState {
    pub state: State,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct ChangePlaylist {
    pub index: usize,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct ChangePosition {
    pub position: f64,
}

#[derive(Message, Serialize)]
#[rtype(result = "()")]
pub struct SyncPosition;

pub struct PlayerActor {
    manager: Addr<SessionManager>,
    session: Arc<str>,
    hb: Instant,
}

impl PlayerActor {
    const INTERVAL: Duration = Duration::from_secs(1);
    const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

    fn new(manager: Addr<SessionManager>, session: Arc<str>) -> Self {
        Self {
            manager,
            session,
            hb: Instant::now(),
        }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(Self::INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > Self::CLIENT_TIMEOUT {
                ctx.stop();
            } else {
                ctx.ping(&[]);
            }
        });
    }
}

impl Actor for PlayerActor {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
        self.manager.do_send(session::PlayerConnect {
            session: self.session.clone(),
            player: ctx.address(),
        });
    }

    fn stopped(&mut self, ctx: &mut Self::Context) {
        self.manager.do_send(session::PlayerDisconnect {
            session: self.session.clone(),
            player: ctx.address(),
        });
    }
}

impl Handler<SyncPosition> for PlayerActor {
    type Result = <SyncPosition as Message>::Result;

    fn handle(&mut self, msg: SyncPosition, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(serde_json::to_string(&BackendMessage::SyncPosition).unwrap());
    }
}

impl Handler<ChangePosition> for PlayerActor {
    type Result = <ChangePosition as Message>::Result;

    fn handle(&mut self, msg: ChangePosition, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(serde_json::to_string(&BackendMessage::ChangePosition(msg.position)).unwrap());
    }
}

impl Handler<ChangeState> for PlayerActor {
    type Result = <ChangeState as Message>::Result;

    fn handle(&mut self, msg: ChangeState, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(serde_json::to_string(&BackendMessage::ChangeState(msg.state)).unwrap());
    }
}
impl Handler<ChangePlaylist> for PlayerActor {
    type Result = <ChangePlaylist as Message>::Result;

    fn handle(&mut self, msg: ChangePlaylist, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(serde_json::to_string(&BackendMessage::ChangePlaylist(msg.index)).unwrap());
    }
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for PlayerActor {
    fn handle(&mut self, item: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match item {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => self.hb = Instant::now(),
            Ok(ws::Message::Text(text)) => {
                let message: PlayerMessage = serde_json::from_str(&text).unwrap();

                match message {
                    PlayerMessage::Seeked => self.manager.do_send(session::Seeked {
                        player: ctx.address(),
                    }),
                    PlayerMessage::StateChanged(state) => {
                        self.manager.do_send(session::StateChanged {
                            session: self.session.clone(),
                            state,
                        })
                    }
                    PlayerMessage::Position(position) => self.manager.do_send(session::Position {
                        session: self.session.clone(),
                        position,
                    }),
                    PlayerMessage::PlaylistChanged(_index) => {
                        self.manager.do_send(session::PlaylistChanged {
                            session: self.session.clone(),
                            index: _index,
                        })
                    }
                }
            }
            _ => {
                ctx.stop();
            }
        }
    }
}

#[get("/player/socket")]
async fn socket(
    manager: Data<Addr<SessionManager>>,
    session: Query<SessionQuery>,
    req: HttpRequest,
    payload: Payload,
) -> Result<HttpResponse> {
    ws::start(
        PlayerActor::new(manager.get_ref().clone(), session.session.clone().into()),
        &req,
        payload,
    )
}

#[get("/join")]
async fn join(manager: Data<Addr<SessionManager>>, query: Query<SessionQuery>) -> Html {
    match manager
        .send(session::GetSession {
            session: query.session.clone().into(),
        })
        .await
        .unwrap()
    {
        Some(session) => Html(
            templates::Player {
                shitposts: &session.shitposts,
                session: &query.session,
            }
            .render()
            .unwrap(),
        ),
        None => Html(
            templates::Error {
                text: "No such session exists",
            }
            .render()
            .unwrap(),
        ),
    }
}

#[get("/host")]
async fn host(config: Data<Config>, session: Query<SessionQuery>) -> Html {
    Html(
        templates::Host {
            folders: &config
                .shitposts
                .iter()
                .map(|folder| folder.split('/').last().unwrap())
                .collect::<Vec<_>>(),
            session: &session.session,
        }
        .render()
        .unwrap(),
    )
}

#[get("/host/submit")]
async fn host_submit(
    manager: Data<Addr<SessionManager>>,
    config: Data<Config>,
    session: Query<SessionConfig>,
    folders: Query<RouletteFolders>,
) -> Html {
    let mut shitposts = config
        .clone()
        .shitposts
        .iter()
        .filter_map(move |folder| {
            let folder_name = folder.split('/').last().unwrap().to_string();

            if folders.0 .0.contains(&folder_name) {
                Some(
                    fs::read_dir(folder)
                        .unwrap()
                        .filter_map(move |entry| {
                            let name = entry.unwrap().file_name().to_string_lossy().to_string();

                            if VALID_FILETYPES
                                .iter()
                                .any(|filetype| name.ends_with(filetype))
                            {
                                Some(Shitpost {
                                    url: format!("/shitposts/{}/{}", folder_name, name,),
                                    title: name,
                                })
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        })
        .flatten()
        .collect::<Vec<_>>();

    shitposts.shuffle(&mut rand::thread_rng());

    shitposts.truncate(session.amount);

    if manager
        .send(session::NewSession {
            session: session.session.clone().into(),
            shitposts: shitposts.clone(),
        })
        .await
        .unwrap()
    {
        Html(
            templates::Player {
                shitposts: &shitposts,
                session: &session.session,
            }
            .render()
            .unwrap(),
        )
    } else {
        Html(
            templates::Error {
                text: "Session already exists",
            }
            .render()
            .unwrap(),
        )
    }
}

#[get("/")]
async fn index() -> Html {
    Html(templates::Index.render().unwrap())
}

#[cfg(test)]
mod tests {
    use crate::player::{PlayerMessage, SyncPosition};

    #[test]
    fn serde_serializations() {
        println!(
            "{}",
            &serde_json::to_string_pretty(&PlayerMessage::Seeked).unwrap()
        );
        println!(
            "{}",
            &serde_json::to_string_pretty(&PlayerMessage::StateChanged(
                crate::player::State::Complete
            ))
            .unwrap()
        );

        println!("{}", &serde_json::to_string_pretty(&SyncPosition).unwrap());
    }
}
