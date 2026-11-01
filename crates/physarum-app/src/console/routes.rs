//! The console's page and buttons.

use std::sync::Arc;

use autumn_web::AppState;
use autumn_web::prelude::*;
use maud::{DOCTYPE, PreEscaped};

use super::conductor::Frame;
use super::{Console, State as LabState};

fn console(state: &AppState) -> AutumnResult<Arc<Console>> {
    state
        .extension::<Console>()
        .ok_or_else(|| AutumnError::service_unavailable_msg("the console is still starting"))
}

/// The stylesheet.
pub const STYLE: &str = include_str!("dish.css");
/// The script.
pub const SCRIPT: &str = include_str!("dish.js");

/// The dish.
#[get("/")]
#[public]
pub async fn index(csrf: Option<CsrfToken>) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Physarum" }
                @if let Some(t) = csrf.as_ref() { meta name="csrf-token" content=(t.token()); }
                style { (PreEscaped(STYLE)) }
                script src="/dish.js" defer {}
            }
            body {
                div class="wrap" {
                    header class="top" {
                        h1 { "Physarum" }
                        span class="tag" { "a slime mold grown in 384 SQLite files" }
                        nav {
                            label class="toggle" {
                                input type="checkbox" id="show-owners";
                                " who serves each cell"
                            }
                            label class="toggle" {
                                input type="checkbox" id="show-agents";
                                " show the agents"
                            }
                            button class="ghost" data-post="/food/clear" { "Take the food away" }
                            button id="grow" class="ghost" data-post="/mold/stop" { "Pause" }
                            button class="ghost" data-post="/reset" data-confirm="Kill every host, wipe the lab and start over?" { "Reset the lab" }
                        }
                    }
                    section class="dish-frame" {
                        canvas id="dish" width="960" height="640" aria-label="The dish: 24 by 16 cells, each one SQLite file. Glowing veins are the slime mold's trail; click to drop food." {}
                        div class="readout" id="readout" {}
                    }
                    p class="lede" {
                        "Every square of this dish is one SQLite file. The mold is 4,000 agents, each a row in the file of the square it is in, "
                        "and the glowing trail they follow is stored in the squares too. No agent ever reads another: each one senses the trail ahead, "
                        "turns toward the strongest, moves, and lays down more. Click to drop food and watch veins grow between the pieces. "
                        "Kill a host and its scatter of squares goes dark; start host-c and the veins come back from the replica."
                    }
                    section class="grid" {
                        div id="hosts" class="hosts" {}
                        div id="headcount" class="panel" {}
                    }
                    section class="panel" {
                        h3 { "Timeline" }
                        ol id="timeline" class="timeline" {}
                    }
                    footer {
                        "Every host is a real autumn-web 0.9 process: a tenant fleet with "
                        code { "[database.fleet.ownership]" } " and " code { "[replication]" }
                        ". The lab lives in " code { "physarum-lab/" } "; each host's log is "
                        code { "physarum-lab/<host>/host.log" } ". The model is Jeff Jones's 2010 agent model of Physarum polycephalum."
                    }
                }
            }
        }
    }
}

/// The last tick's dish.
///
/// # Errors
///
/// 503 while the console starts.
#[get("/_frame.json")]
#[public]
pub async fn frame(State(state): State<AppState>) -> AutumnResult<Json<Frame>> {
    Ok(Json((*console(&state)?.frame()).clone()))
}

/// The hosts and the timeline.
///
/// # Errors
///
/// 503 while the console starts.
#[get("/_state.json")]
#[public]
pub async fn lab_state(State(state): State<AppState>) -> AutumnResult<Json<LabState>> {
    Ok(Json(console(&state)?.state()))
}

/// A host button.
///
/// # Errors
///
/// 409 if the action does not apply to the host as it is now.
#[post("/hosts/{name}/{action}")]
#[public]
pub async fn host_action(
    State(state): State<AppState>,
    Path((name, action)): Path<(String, String)>,
) -> AutumnResult<Json<LabState>> {
    let c = console(&state)?;
    c.act(&name, &action)
        .map_err(|e| AutumnError::conflict_msg(e.to_string()))?;
    Ok(Json(c.state()))
}

/// Let the mold grow, or pause it.
///
/// # Errors
///
/// 400 for anything but `start` or `stop`.
#[post("/mold/{on}")]
#[public]
pub async fn mold(
    State(state): State<AppState>,
    Path(on): Path<String>,
) -> AutumnResult<Json<LabState>> {
    let c = console(&state)?;
    match on.as_str() {
        "start" => c.set_growing(true),
        "stop" => c.set_growing(false),
        other => {
            return Err(AutumnError::bad_request_msg(format!(
                "no mold action {other}"
            )));
        }
    }
    Ok(Json(c.state()))
}

/// Drop food at a point of the dish, in units (0–960, 0–640).
///
/// # Errors
///
/// 400 for a point that is not a number.
#[post("/food/{x}/{y}")]
#[public]
pub async fn food(
    State(state): State<AppState>,
    Path((x, y)): Path<(f32, f32)>,
) -> AutumnResult<Json<LabState>> {
    if !(x.is_finite() && y.is_finite()) {
        return Err(AutumnError::bad_request_msg(
            "food goes at a point of the dish",
        ));
    }
    let c = console(&state)?;
    c.add_food(x, y).await;
    Ok(Json(c.state()))
}

/// Take every piece of food away.
///
/// # Errors
///
/// 503 while the console starts.
#[post("/food/clear")]
#[public]
pub async fn clear_food(State(state): State<AppState>) -> AutumnResult<Json<LabState>> {
    let c = console(&state)?;
    c.clear_food().await;
    Ok(Json(c.state()))
}

/// Start over.
///
/// # Errors
///
/// 500 if the lab directory could not be cleared.
#[post("/reset")]
#[public]
pub async fn reset(State(state): State<AppState>) -> AutumnResult<Json<LabState>> {
    let c = console(&state)?;
    c.reset()
        .await
        .map_err(|e| AutumnError::internal_server_error_msg(e.to_string()))?;
    c.set_growing(true);
    Ok(Json(c.state()))
}

/// The script.
#[get("/dish.js")]
#[public]
pub async fn script() -> impl IntoResponse {
    (
        [
            ("content-type", "text/javascript; charset=utf-8"),
            ("cache-control", "no-cache"),
        ],
        SCRIPT,
    )
}
