use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

use super::McEvent;

pub async fn serve(mut socket: WebSocket, mut rx: broadcast::Receiver<McEvent>) {
    // Push-only websocket: server sends refresh events.
    loop {
        tokio::select! {
            evt = rx.recv() => {
                match evt {
                    Ok(McEvent::Refresh{at, reason}) => {
                        let payload = serde_json::json!({"type":"refresh","at":at,"reason":reason});
                        let _ = socket.send(Message::Text(payload.to_string())).await;
                    }
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => { /* ignore */ }
                    Some(Err(_)) => break,
                }
            }
        }
    }
}
