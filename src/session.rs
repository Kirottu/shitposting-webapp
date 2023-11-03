use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use actix::{Actor, Addr, Context, Handler, Message, MessageResponse};

use crate::{
    player::{self, PlayerActor},
    Shitpost,
};

#[derive(Message)]
#[rtype(result = "()")]
pub struct StateChanged {
    pub session: Arc<str>,
    pub state: player::State,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Seeked {
    pub player: Addr<PlayerActor>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PlaylistChanged {
    pub session: Arc<str>,
    pub index: usize,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Position {
    pub session: Arc<str>,
    pub position: f64,
}

#[derive(Message)]
#[rtype(result = "bool")]
pub struct NewSession {
    pub session: Arc<str>,
    pub shitposts: Vec<Shitpost>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerConnect {
    pub session: Arc<str>,
    pub player: Addr<PlayerActor>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PlayerDisconnect {
    pub session: Arc<str>,
    pub player: Addr<PlayerActor>,
}

#[derive(Message)]
#[rtype(result = "Option<Session>")]
pub struct GetSession {
    pub session: Arc<str>,
}

#[derive(MessageResponse, Clone)]
pub struct Session {
    pub shitposts: Vec<Shitpost>,
    pub state: player::State,
    pub playlist_index: usize,
    players: Vec<Addr<PlayerActor>>,
}

pub struct SessionManager {
    sessions: HashMap<Arc<str>, Session>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }
}

impl Actor for SessionManager {
    type Context = Context<Self>;
}

impl Handler<NewSession> for SessionManager {
    type Result = <NewSession as Message>::Result;

    /// Returns false if the session already exists
    fn handle(&mut self, msg: NewSession, ctx: &mut Self::Context) -> Self::Result {
        if let Entry::Vacant(e) = self.sessions.entry(msg.session.clone()) {
            tracing::info!(r#"Created session "{}""#, msg.session);
            e.insert(Session {
                shitposts: msg.shitposts,
                state: player::State::Paused,
                playlist_index: 0,
                players: Vec::new(),
            });
            true
        } else {
            false
        }
    }
}

impl Handler<PlayerConnect> for SessionManager {
    type Result = <PlayerConnect as Message>::Result;

    fn handle(&mut self, msg: PlayerConnect, ctx: &mut Self::Context) -> Self::Result {
        if let Some(session) = self.sessions.get_mut(&msg.session) {
            msg.player.do_send(player::ChangeState {
                state: session.state,
            });
            msg.player.do_send(player::ChangePlaylist {
                index: session.playlist_index,
            });

            session.players.push(msg.player);

            session.players[0].do_send(player::SyncPosition);
        }
    }
}

impl Handler<PlayerDisconnect> for SessionManager {
    type Result = <PlayerDisconnect as Message>::Result;

    fn handle(&mut self, msg: PlayerDisconnect, ctx: &mut Self::Context) -> Self::Result {
        if if let Some(session) = self.sessions.get_mut(&msg.session) {
            session.players.retain(|player| *player != msg.player);
            session.players.is_empty()
        } else {
            false
        } {
            tracing::info!(r#"Session "{}" removed"#, msg.session);
            self.sessions.remove(&msg.session);
        }
    }
}

impl Handler<StateChanged> for SessionManager {
    type Result = <StateChanged as Message>::Result;

    fn handle(&mut self, msg: StateChanged, ctx: &mut Self::Context) -> Self::Result {
        if let Some(session) = self.sessions.get_mut(&msg.session) {
            session.state = msg.state;
            for player in &session.players {
                player.do_send(player::ChangeState { state: msg.state });
            }
        }
    }
}

impl Handler<PlaylistChanged> for SessionManager {
    type Result = <PlaylistChanged as Message>::Result;

    fn handle(&mut self, msg: PlaylistChanged, ctx: &mut Self::Context) -> Self::Result {
        if let Some(session) = self.sessions.get_mut(&msg.session) {
            session.playlist_index = msg.index;
            for player in &session.players {
                player.do_send(player::ChangePlaylist { index: msg.index });
            }
        }
    }
}

impl Handler<Seeked> for SessionManager {
    type Result = <Seeked as Message>::Result;

    fn handle(&mut self, msg: Seeked, ctx: &mut Self::Context) -> Self::Result {
        msg.player.do_send(player::SyncPosition);
    }
}

impl Handler<Position> for SessionManager {
    type Result = <Position as Message>::Result;

    fn handle(&mut self, msg: Position, ctx: &mut Self::Context) -> Self::Result {
        if let Some(session) = self.sessions.get_mut(&msg.session) {
            for player in &session.players {
                player.do_send(player::ChangePosition {
                    position: msg.position,
                });
            }
        }
    }
}

impl Handler<GetSession> for SessionManager {
    type Result = <GetSession as Message>::Result;

    fn handle(&mut self, msg: GetSession, ctx: &mut Self::Context) -> Self::Result {
        self.sessions.get(&msg.session).cloned()
    }
}
