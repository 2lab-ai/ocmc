pub mod handlers;

use leptos::*;
use leptos_meta::*;
use leptos_router::*;

pub fn leptos_options() -> leptos::LeptosOptions {
    leptos::LeptosOptions::builder()
        .output_name("mission_control")
        .site_root("site")
        .site_pkg_dir("pkg")
        .build()
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Stylesheet id="leptos" href="/pkg/mission_control.css"/>
        <Title text="Mission Control"/>
        <Router>
            <main style="font-family: ui-sans-serif, system-ui; padding: 16px;">
                <Routes>
                    <Route path="/" view=Dashboard/>
                </Routes>
            </main>
        </Router>
    }
}

#[component]
fn Dashboard() -> impl IntoView {
    let (snapshot, set_snapshot) = create_signal::<Option<crate::mc::KanbanSnapshot>>(None);
    let (err, set_err) = create_signal::<Option<String>>(None);

    let load = move || {
        spawn_local(async move {
            match reqwasm::http::Request::get("/api/kanban").send().await {
                Ok(resp) if resp.status() == 200 => {
                    let json = resp.json::<crate::mc::KanbanSnapshot>().await;
                    match json {
                        Ok(s) => { set_snapshot.set(Some(s)); set_err.set(None); }
                        Err(e) => set_err.set(Some(format!("json parse: {e:?}")))
                    }
                }
                Ok(resp) if resp.status() == 401 => {
                    set_err.set(Some("unauthorized".to_string()));
                }
                Ok(resp) => {
                    set_err.set(Some(format!("http {}", resp.status())));
                }
                Err(e) => {
                    set_err.set(Some(format!("fetch err: {e:?}")));
                }
            }
        });
    };

    create_effect(move |_| {
        load();

        // websocket refresh
        spawn_local(async move {
            if let Ok(ws) = gloo_net::websocket::futures::WebSocket::open("ws://localhost:3000/ws") {
                let (_write, mut read) = ws.split();
                while let Some(Ok(msg)) = read.next().await {
                    if let gloo_net::websocket::Message::Text(_) = msg {
                        load();
                    }
                }
            }
        });
    });

    view! {
        <h1 style="margin: 0 0 12px 0;">"Mission Control"</h1>
        <Show when=move || err.get().as_deref() == Some("unauthorized") fallback=move || view!{}>
            <p>"로그인이 필요합니다: " <a href="/login">"/login"</a></p>
        </Show>

        <Show when=move || err.get().is_some() fallback=move || view!{}>
            <pre style="background:#fee; padding: 8px;">{move || err.get().unwrap_or_default()}</pre>
        </Show>

        <Show when=move || snapshot.get().is_some() fallback=move || view!{ <p>"Loading…"</p> }>
            {move || snapshot.get().map(render_snapshot)}
        </Show>
    }
}

fn render_snapshot(s: crate::mc::KanbanSnapshot) -> impl IntoView {
    let lanes = ["Backlog", "Ready", "Doing", "Blocked", "Done"];

    view! {
        <section>
            <h2>"Agents"</h2>
            <div style="display:flex; gap: 8px; flex-wrap: wrap;">
                {s.agents.into_iter().map(|a| view!{
                    <div style="border:1px solid #ddd; padding:8px; border-radius:8px; min-width:180px;">
                        <div><b>{a.display_name}</b> " (" {a.state} ")"</div>
                        <div style="color:#666; font-size: 12px;">{a.current_card_id.unwrap_or_else(|| "waiting-room".into())}</div>
                    </div>
                }).collect_view()}
            </div>
        </section>

        <section style="margin-top:16px;">
            <h2>"Tasks (bd)"</h2>
            <div style="display:flex; gap:12px; align-items:flex-start; overflow:auto;">
                {lanes.into_iter().map(|lane| {
                    let cards = s.tasks.iter().filter(|t| t.lane == lane).cloned().collect::<Vec<_>>();
                    view!{
                        <div style="min-width:280px; border:1px solid #ddd; border-radius:12px; padding:8px;">
                            <div style="font-weight:700; margin-bottom:8px;">{lane}</div>
                            {cards.into_iter().map(|t| view!{
                                <div style="border:1px solid #eee; border-radius:10px; padding:8px; margin-bottom:8px; background:#fff;">
                                    <div style="font-weight:600;">{t.id.clone()} " " {t.title}</div>
                                    <div style="font-size:12px; color:#666;">{"assignee: "}{t.assignee.unwrap_or_else(|| "-".into())}</div>
                                </div>
                            }).collect_view()}
                        </div>
                    }
                }).collect_view()}

                <div style="min-width:320px; border:1px solid #ddd; border-radius:12px; padding:8px;">
                    <div style="font-weight:700; margin-bottom:8px;">"Cron"</div>
                    {s.cron.into_iter().map(|c| view!{
                        <div style="border:1px solid #eee; border-radius:10px; padding:8px; margin-bottom:8px; background:#fff;">
                            <div style="font-weight:600;">{c.name}</div>
                            <div style="font-size:12px; color:#666;">{c.id} " — " {c.schedule} " — enabled=" {c.enabled}</div>
                        </div>
                    }).collect_view()}
                </div>
            </div>
        </section>
    }
}
