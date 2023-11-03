use std::fs;

use actix::Actor;
use actix_files::Files;
use actix_web::{body::BoxBody, web::Data, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;
use session::SessionManager;

mod player;
mod session;

#[derive(Deserialize)]
struct Config {
    shitposts: Vec<String>,
    bind: String,
}

#[derive(Clone)]
pub struct Shitpost {
    title: String,
    url: String,
}

struct Html(String);

impl Responder for Html {
    type Body = BoxBody;

    fn respond_to(self, _req: &actix_web::HttpRequest) -> actix_web::HttpResponse<Self::Body> {
        HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(self.0)
    }
}

#[actix_web::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let config: Config = ron::de::from_bytes(&fs::read("config.ron").unwrap()).unwrap();

    let config = Data::new(config);
    let bind = config.bind.clone();

    let manager = Data::new(SessionManager::new().start());

    HttpServer::new(move || {
        let mut app = App::new()
            .service(player::host)
            .service(player::host_submit)
            .service(player::join)
            .service(player::index)
            .service(player::socket)
            .service(Files::new("/static", "./static"))
            .app_data(manager.clone())
            .app_data(config.clone());

        for folder in &config.shitposts {
            app = app.service(Files::new(
                &format!("/shitposts/{}", folder.split('/').last().unwrap()),
                folder,
            ));
        }
        app
    })
    .bind(bind)
    .unwrap()
    .run()
    .await
    .unwrap();
}
