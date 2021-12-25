use std::collections::HashMap;
use std::sync::{
  atomic::{AtomicUsize, Ordering},
  Arc,
};

use futures::{FutureExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::ws::{Message, WebSocket};
use warp::Filter;

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
static INDEX_HTML: &str = std::include_str!("../static/index.html");

type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Result<Message, warp::Error>>>>>;

#[tokio::main]
async fn main() {
  let users = Users::default();
  let users = warp::any().map(move || users.clone());

  let index = warp::path::end().map(|| warp::reply::html(INDEX_HTML));

  let chat = warp::path("chat")
    .and(warp::ws())
    .and(users)
    .map(|ws: warp::ws::Ws, users| ws.on_upgrade(move |socket| user_connected(socket, users)));

  let routes = index.or(chat);

  warp::serve(routes).run(([127, 0, 0, 1], 5000)).await;
}

async fn user_connected(ws: WebSocket, users: Users) {
  let user_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);

  eprintln!("new chat user: {}", user_id);

  let (user_ws_tx, mut user_ws_rx) = ws.split();

  let (tx, rx) = mpsc::unbounded_channel();
  let rx = UnboundedReceiverStream::new(rx);

  tokio::task::spawn(rx.forward(user_ws_tx).map(|result| {
    if let Err(e) = result {
      eprintln!("websocket send error: {}", e);
    }
  }));

  users.write().await.insert(user_id, tx);

  let users_copy = users.clone();

  while let Some(result) = user_ws_rx.next().await {
    let msg = match result {
      Ok(msg) => msg,
      Err(e) => {
        eprintln!("websocket error(uid={}): {}", user_id, e);
        break;
      }
    };

    user_message(user_id, msg, &users).await;
  }
  user_disconnected(user_id, &users_copy).await;
}

async fn user_message(user_id: usize, msg: Message, users: &Users) {
  let msg = if let Ok(s) = msg.to_str() {
    s
  } else {
    return;
  };

  let new_msg = format!("<User#{}>: {}", user_id, msg);

  for (&uid, tx) in users.read().await.iter() {
    if user_id != uid {
      if let Err(_disconnected) = tx.send(Ok(Message::text(new_msg.clone()))) {}
    }
  }
}

async fn user_disconnected(user_id: usize, users: &Users) {
  eprintln!("user {} disconnected", user_id);
  users.write().await.remove(&user_id);
}
