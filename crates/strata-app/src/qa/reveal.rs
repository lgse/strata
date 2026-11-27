// SPDX-License-Identifier: MIT

//! `reveal` and `reveal_control` on the Rune QA backend, ported from `src/qa/reveal.rs`.

use std::time::Instant;

use serde_json::json;
use strata_qa::{
    Backend as _, Check, DEFAULT_WAIT, Failure, LookupError, Outcome, POLL_INTERVAL, Predicate,
    REVEAL_SCROLL_ATTEMPTS, Scope, Target, WHEEL_CLICKS, Wanted, inside, on_screen, screen_rect,
};

use super::{backend::Rune, host::Host, tree::Node};

impl<H: Host> Rune<H> {
    async fn presented_control(
        &self,
        scope: &Scope<'_, Self>,
        target: &Target,
        message: String,
    ) -> Result<(Node, Node), Outcome> {
        let wanted =
            Wanted::of(target).map_err(|error| Outcome::error(error.message().to_owned()))?;
        let deadline = Instant::now() + DEFAULT_WAIT;
        let node = loop {
            if let Some(node) = wanted.find_presented(self, &scope.window) {
                break node;
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "presented": target }),
                    actual: json!(false),
                    message: Some(message),
                };
                return Err(scope.timed_out(failure, DEFAULT_WAIT));
            }
            self.sleep(POLL_INTERVAL).await;
        };
        let scroll = std::iter::successors(self.parent(&node), |current| self.parent(current))
            .find(|ancestor| self.role(ancestor) == "scroll pane")
            .ok_or_else(|| Outcome::error("the control has no scroll pane around it".to_owned()))?;
        self.await_control_at_rest(scope, &node, (deadline, &message))
            .await?;
        Ok((node, scroll))
    }

    async fn await_control_at_rest(
        &self,
        scope: &Scope<'_, Self>,
        node: &Node,
        (deadline, message): (Instant, &str),
    ) -> Result<(), Outcome> {
        loop {
            let before = self.node_pixels(node);
            self.settle(&scope.window).await;
            let after = self.node_pixels(node);
            match (before, after) {
                (Ok(before), Ok(after)) if has_settled_bounds(before, after) => return Ok(()),
                (_, Err(LookupError::Broken(error))) => return Err(Outcome::error(error)),
                _ if Instant::now() >= deadline => {
                    return Err(Outcome::error(format!(
                        "{message}: the control never came to rest"
                    )));
                }
                _ => self.sleep(POLL_INTERVAL).await,
            }
        }
    }

    fn node_pixels(&self, node: &Node) -> Result<[i64; 4], LookupError> {
        screen_rect(self, node).map(|rect| rect.pixels())
    }

    pub(super) async fn reveal_control(&self, scope: &Scope<'_, Self>, target: &Target) -> Outcome {
        let name = control_name(target);
        let (node, scroll) = match self
            .presented_control(scope, target, format!("{name} control"))
            .await
        {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        let deadline = Instant::now() + DEFAULT_WAIT;
        loop {
            let (bounds, viewport) = match (self.node_pixels(&node), self.node_pixels(&scroll)) {
                (Ok(bounds), Ok(viewport)) => (bounds, viewport),
                (Err(LookupError::Broken(error)), _) | (_, Err(LookupError::Broken(error))) => {
                    return Outcome::error(error);
                }
                _ if Instant::now() >= deadline => {
                    return Outcome::error(format!("{name} has no bounds"));
                }
                _ => {
                    self.sleep(POLL_INTERVAL).await;
                    continue;
                }
            };
            if inside(bounds, viewport, None) {
                break;
            }
            if Instant::now() >= deadline {
                let failure = Failure {
                    predicate: json!({ "reveal_control": target }),
                    actual: json!({ "bounds": bounds, "viewport": viewport }),
                    message: Some(format!("reachable {name} control")),
                };
                return scope.timed_out(failure, DEFAULT_WAIT);
            }
            if let Err(error) = wheel_viewport(scope, bounds, viewport).await {
                return Outcome::error(error);
            }
            self.sleep(POLL_INTERVAL).await;
        }
        scope
            .settle_target(&presented(target), DEFAULT_WAIT, None)
            .await
    }

    pub(super) async fn reveal(&self, scope: &Scope<'_, Self>, target: &Target) -> Outcome {
        let criteria = target_criteria(target);
        let (node, scroll) = match self
            .presented_control(scope, target, format!("a control matching {criteria}"))
            .await
        {
            Ok(found) => found,
            Err(outcome) => return outcome,
        };
        match self.in_scroll_pane(&node, &scroll, &scope.window) {
            Err(error) => return Outcome::error(error),
            Ok(true) => return Outcome::default(),
            Ok(false) => {}
        }
        let window = match self.node_pixels(&scope.window) {
            Ok(window) => window,
            Err(error) => return Outcome::error(error.message().to_owned()),
        };
        let pane = self.node_pixels(&scroll).unwrap_or(window);
        let centre = (
            window[0] + window[2].div_euclid(2),
            window[1] + window[3].div_euclid(2),
        );
        for _ in 0..REVEAL_SCROLL_ATTEMPTS {
            match self.in_scroll_pane(&node, &scroll, &scope.window) {
                Err(error) => return Outcome::error(error),
                Ok(true) => {
                    return scope
                        .settle_target(&presented(target), DEFAULT_WAIT, None)
                        .await;
                }
                Ok(false) => {}
            }
            let rect = match self.node_pixels(&node) {
                Ok(rect) => rect,
                Err(error) => return Outcome::error(error.message().to_owned()),
            };
            let below = rect[1] > centre.1;
            let distance = if below {
                (rect[1] + rect[3] - (pane[1] + pane[3])).max(0)
            } else {
                (pane[1] - rect[1]).max(0)
            };
            #[expect(
                clippy::cast_sign_loss,
                clippy::cast_possible_truncation,
                reason = "non-negative distance clamped to WHEEL_CLICKS..=12"
            )]
            let clicks = (distance / 80).clamp(i64::from(WHEEL_CLICKS), 12) as u32;
            if let Err(error) = scope.wheel_at(centre, clicks, below, &[]).await {
                return Outcome::error(error);
            }
        }
        self.await_on_screen(scope, target, &node).await
    }

    fn in_scroll_pane(&self, node: &Node, scroll: &Node, window: &Node) -> Result<bool, String> {
        match (
            self.node_pixels(node),
            self.node_pixels(scroll),
            self.node_pixels(window),
        ) {
            (Ok(bounds), Ok(pane), Ok(win)) => Ok(on_screen(bounds, win)
                && bounds[1] >= pane[1]
                && bounds[1] + bounds[3] <= pane[1] + pane[3]),
            (Err(LookupError::Broken(error)), _, _)
            | (_, Err(LookupError::Broken(error)), _)
            | (_, _, Err(LookupError::Broken(error))) => Err(error),
            _ => Ok(false),
        }
    }

    async fn await_on_screen(
        &self,
        scope: &Scope<'_, Self>,
        target: &Target,
        node: &Node,
    ) -> Outcome {
        let check = Predicate {
            check: Check::OnScreen(presented(target)),
            dir: None,
        };
        let deadline = Instant::now() + DEFAULT_WAIT;
        loop {
            match scope.check(&check) {
                Err(error) => return Outcome::error(error),
                Ok(Ok(())) => return Outcome::default(),
                Ok(Err(mut failure)) if Instant::now() >= deadline => {
                    failure.message = Some(format!(
                        "<{} {}> to be scrolled into view",
                        self.role(node),
                        python_repr(&self.name(node))
                    ));
                    return scope.timed_out(failure, DEFAULT_WAIT);
                }
                Ok(Err(_)) => self.sleep(POLL_INTERVAL).await,
            }
        }
    }
}

fn has_settled_bounds(before: [i64; 4], after: [i64; 4]) -> bool {
    let sized = after[2] > 0 && after[3] > 0;
    before == after && sized
}

async fn wheel_viewport<B: strata_qa::Backend>(
    scope: &Scope<'_, B>,
    bounds: [i64; 4],
    viewport: [i64; 4],
) -> Result<(), String> {
    let [view_x, view_y, view_width, view_height] = viewport;
    let at = (view_x + view_width - 16, view_y + view_height.div_euclid(2));
    let below = bounds[1] + bounds[3] - (view_y + view_height);
    let above = view_y - bounds[1];
    let distance = below.max(above);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "Python's int() of a true division of pixel counts"
    )]
    let steps = (distance as f64 / view_height as f64).trunc() as i64 * 3;
    let clicks = steps.clamp(1, 12);
    #[expect(clippy::cast_sign_loss, reason = "clamped to 1..=12")]
    scope.wheel_at(at, clicks as u32, below > 0, &[]).await
}

fn presented(target: &Target) -> Target {
    let mut target = target.clone();
    if let Target::Role { rendered, .. } = &mut target {
        *rendered = false;
    }
    target
}

fn control_name(target: &Target) -> String {
    match target {
        Target::Role {
            name: Some(name), ..
        } => name.clone(),
        other => other.to_json().to_string(),
    }
}

fn target_criteria(target: &Target) -> String {
    match target {
        Target::Role {
            role,
            name: Some(name),
            ..
        } => format!(
            "{{'role': {}, 'name': {}}}",
            python_repr(role),
            python_repr(name)
        ),
        other => other.to_json().to_string(),
    }
}

fn python_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut printed = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => printed.push_str("\\\\"),
            '\n' => printed.push_str("\\n"),
            '\t' => printed.push_str("\\t"),
            ch if ch == quote => {
                printed.push('\\');
                printed.push(ch);
            }
            ch => printed.push(ch),
        }
    }
    printed.push(quote);
    printed
}
