//! When the bridge may exit on its own.
//!
//! The bridge has no window and no tray icon, so it has to know when to leave.
//! Upstream's rule was "45 s after the last browser tab closes", which fits a
//! dashboard on the racing PC and breaks a remote one: the engineer's tablet
//! goes to sleep, the count drops to zero, and the bridge on the driver's PC
//! quits in the middle of a stint with nobody there to restart it.
//!
//! The rule here: the bridge is **idle** only while no client is connected
//! *and* the game is not running, and it exits after staying idle for a full
//! grace period. A running game keeps it up indefinitely; so does any client.
//! The grace period covers restarting the game between sessions.
//!
//! As upstream, nothing happens until a first client has connected: a bridge
//! started with Windows waits for as long as it takes.

use std::time::Duration;

use tokio::sync::watch;

/// How long the bridge stays up with no client and no game.
pub const IDLE_GRACE: Duration = Duration::from_secs(600);

/// Resolve once the bridge has been idle for `grace`. See the module docs.
///
/// Also resolves if either channel closes, which only happens when the rest
/// of the bridge is already going down.
pub async fn wait_until_idle(
    mut clients: watch::Receiver<usize>,
    mut game_running: watch::Receiver<bool>,
    grace: Duration,
) {
    // Phase 1: wait for the first client ever. The count starts at 0 and can
    // only leave it through a connect, so *any* change counts — including a
    // client that came and went before this task was first polled, which
    // comparing the value alone would miss.
    if *clients.borrow() == 0 && clients.changed().await.is_err() {
        return;
    }

    // Phase 2: leave after one uninterrupted grace period of idleness.
    loop {
        let idle = *clients.borrow() == 0 && !*game_running.borrow();
        if idle {
            tokio::select! {
                _ = tokio::time::sleep(grace) => return,
                r = clients.changed() => if r.is_err() { return },
                r = game_running.changed() => if r.is_err() { return },
            }
        } else {
            tokio::select! {
                r = clients.changed() => if r.is_err() { return },
                r = game_running.changed() => if r.is_err() { return },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    const GRACE: Duration = Duration::from_millis(80);
    /// Comfortably longer than GRACE: "it would have fired by now".
    const LONG: Duration = Duration::from_millis(400);

    struct Rig {
        clients: watch::Sender<usize>,
        game: watch::Sender<bool>,
        done: tokio::task::JoinHandle<()>,
    }

    fn rig() -> Rig {
        let (clients, c_rx) = watch::channel(0usize);
        let (game, g_rx) = watch::channel(false);
        let done = tokio::spawn(wait_until_idle(c_rx, g_rx, GRACE));
        Rig { clients, game, done }
    }

    #[tokio::test]
    async fn never_exits_before_a_first_client() {
        let mut r = rig();
        assert!(timeout(LONG, &mut r.done).await.is_err());
    }

    #[tokio::test]
    async fn exits_after_the_last_client_leaves_with_no_game() {
        let mut r = rig();
        r.clients.send(1).unwrap();
        r.clients.send(0).unwrap();
        assert!(timeout(LONG, &mut r.done).await.is_ok());
    }

    #[tokio::test]
    async fn a_running_game_keeps_it_alive_with_no_clients() {
        let mut r = rig();
        r.game.send(true).unwrap();
        r.clients.send(1).unwrap();
        r.clients.send(0).unwrap();
        assert!(timeout(LONG, &mut r.done).await.is_err());
        // The game closes: now the grace period runs, and it exits.
        r.game.send(false).unwrap();
        assert!(timeout(LONG, &mut r.done).await.is_ok());
    }

    #[tokio::test]
    async fn a_reconnect_inside_the_grace_period_cancels_the_exit() {
        let mut r = rig();
        r.clients.send(1).unwrap();
        r.clients.send(0).unwrap();
        tokio::time::sleep(GRACE / 2).await;
        r.clients.send(1).unwrap();
        assert!(timeout(LONG, &mut r.done).await.is_err());
    }

    #[tokio::test]
    async fn the_grace_period_restarts_after_each_interruption() {
        let mut r = rig();
        r.clients.send(1).unwrap();
        r.clients.send(0).unwrap();
        tokio::time::sleep(GRACE / 2).await;
        // A game start/stop blip must not let the old timer carry over.
        r.game.send(true).unwrap();
        r.game.send(false).unwrap();
        assert!(timeout(GRACE * 3 / 4, &mut r.done).await.is_err());
        assert!(timeout(LONG, &mut r.done).await.is_ok());
    }
}
