use std::{
    fs,
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
use serde::{Deserialize, Serialize};

use crate::{
    session::{self, SessionManager},
    Config, Html, RouletteFolders, SessionConfig, Shitpost, VALID_FILETYPES,
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
        pub folders: &'a [String],
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
    session: String,
    hb: Instant,
}

impl PlayerActor {
    const INTERVAL: Duration = Duration::from_secs(1);
    const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

    fn new(manager: Addr<SessionManager>, session: String) -> Self {
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
        PlayerActor::new(manager.get_ref().clone(), session.session.clone()),
        &req,
        payload,
    )
}

#[get("/join")]
async fn join() -> Html {
    Html(templates::Join.render().unwrap())
}

#[get("/join/submit")]
async fn join_submit(manager: Data<Addr<SessionManager>>, query: Query<SessionQuery>) -> Html {
    let session = manager
        .send(session::GetSession {
            session: query.session.clone(),
        })
        .await
        .unwrap()
        .unwrap();

    Html(
        templates::Player {
            shitposts: &session.shitposts,
            session: &query.session,
        }
        .render()
        .unwrap(),
    )
}

#[get("/host")]
async fn host(config: Data<Config>) -> Html {
    Html(
        templates::Host {
            folders: &fs::read_dir(&config.shitposts)
                .unwrap()
                .filter_map(|entry| {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_dir() {
                        Some(entry.file_name().to_string_lossy().to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>(),
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
    let mut shitposts = fs::read_dir(&config.shitposts)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.unwrap();

            let config = config.clone();

            if entry.file_type().unwrap().is_dir() {
                let folder = entry.file_name().to_string_lossy().to_string();
                if folders.0 .0.contains(&folder) {
                    return Some(
                        fs::read_dir(entry.path())
                            .unwrap()
                            .filter_map(move |entry| {
                                let name = entry.unwrap().file_name().to_string_lossy().to_string();

                                if VALID_FILETYPES
                                    .iter()
                                    .any(|filetype| name.ends_with(filetype))
                                {
                                    Some(Shitpost {
                                        url: format!(
                                            "{}/shitposts/{}/{}",
                                            config.host, folder, name,
                                        ),
                                        title: name,
                                    })
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>(),
                    );
                }
            }

            None
        })
        .flatten()
        .collect::<Vec<_>>();

    shitposts.shuffle(&mut rand::thread_rng());

    shitposts.truncate(session.amount);

    if manager
        .send(session::NewSession {
            session: session.session.to_string(),
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
