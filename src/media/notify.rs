//! macOS distributed notifications, for a player that posts one when it
//! changes (the Spotify app: `com.spotify.client.PlaybackStateChanged` on
//! play, pause and track change; Apple Music's `com.apple.Music.playerInfo`,
//! lava-75z.26). Each one [`Nudge`]s the media worker into an
//! immediate poll, so a change made in the player shows without polling
//! fast.
//!
//! macOS delivers distributed notifications only to a process's main-thread
//! run loop, and LavaTUI's main thread is drawing. So a small helper
//! listens: a JavaScript for Automation `osascript` ([`script`]) that
//! prints `changed` per notification and exits when its stdin closes
//! (LavaTUI gone, however it went), or at the latest a second after its
//! parent is no longer LavaTUI. A reader thread turns the lines into
//! nudges. Started by the worker (never on the frame path); dropping the
//! [`Watcher`] ends it.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::thread;

use super::worker::Nudge;

/// The helper: an observer for `names` on the distributed notification
/// center, delivered immediately even while LavaTUI is in the background,
/// `ready` once listening, `changed` per notification.
pub fn script(names: &[&str]) -> String {
    let names: Vec<String> = names.iter().map(|n| format!("{n:?}")).collect();
    format!(
        "ObjC.import('Foundation');\n\
         ObjC.import('stdlib');\n\
         ObjC.import('unistd');\n\
         const parent = $.getppid();\n\
         const say = t => $.NSFileHandle.fileHandleWithStandardOutput\n\
           .writeData($(t + '\\n').dataUsingEncoding($.NSUTF8StringEncoding));\n\
         ObjC.registerSubclass({{\n\
           name: 'LavatuiWatch',\n\
           methods: {{\n\
             'seen:': {{ types: ['void', ['id']], implementation: function (n) {{ say('changed'); }} }},\n\
             'input:': {{ types: ['void', ['id']], implementation: function (n) {{\n\
               const data = n.userInfo.objectForKey($.NSFileHandleNotificationDataItem);\n\
               if (data.length == 0) {{ $.exit(0); }}\n\
               n.object.readInBackgroundAndNotify;\n\
             }} }},\n\
           }},\n\
         }});\n\
         const w = $.LavatuiWatch.alloc.init;\n\
         const center = $.NSDistributedNotificationCenter.defaultCenter;\n\
         [{}].forEach(n => center.addObserverSelectorNameObjectSuspensionBehavior(w, 'seen:', n, $(), 4));\n\
         const stdin = $.NSFileHandle.fileHandleWithStandardInput;\n\
         $.NSNotificationCenter.defaultCenter.addObserverSelectorNameObject(\n\
           w, 'input:', $.NSFileHandleReadCompletionNotification, stdin);\n\
         stdin.readInBackgroundAndNotify;\n\
         say('ready');\n\
         while ($.getppid() == parent) $.NSRunLoop.currentRunLoop\n\
           .runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(1));\n",
        names.join(", ")
    )
}

/// The running helper. Dropping it closes its stdin (it exits) and makes
/// sure.
pub struct Watcher {
    child: Child,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Listen for `names`, nudging on each, until the [`Watcher`] is dropped
/// or the worker is gone. `None` if the helper can't start (no nudges:
/// polling still catches everything, only later).
pub fn watch(names: &[&str], nudge: Nudge) -> Option<Watcher> {
    let mut child = Command::new("/usr/bin/osascript")
        .args(["-l", "JavaScript", "-e", &script(names)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let out = child.stdout.take()?;
    let reader = thread::Builder::new()
        .name("lavatui-media-events".into())
        .spawn(move || {
            crate::thread_qos::worker();
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if line == "changed" && !nudge.changed() {
                    break;
                }
            }
        });
    if reader.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    Some(Watcher { child })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Post `name` as another program would, through a second helper.
    fn post(name: &str) {
        let js = format!(
            "ObjC.import('Foundation'); $.NSDistributedNotificationCenter.defaultCenter\
             .postNotificationNameObjectUserInfoDeliverImmediately({name:?}, $(), $(), true);"
        );
        let ok = Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", &js])
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "posting {name}");
    }

    #[test]
    fn a_notification_nudges_the_worker_and_the_helper_goes_with_the_watcher() {
        // A name of our own: nothing else posts it, and Spotify never sees it.
        let name = format!("dev.lavatui.test.{}", std::process::id());
        let (nudge, rx) = Nudge::channel();
        let watcher = watch(&[&name], nudge).expect("osascript");
        let pid = watcher.child.id();
        // Listening takes the helper a moment to set up: post until heard.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            post(&name);
            if rx.recv_timeout(Duration::from_millis(500)).is_ok() {
                break;
            }
            assert!(Instant::now() < deadline, "never heard {name}");
        }
        drop(watcher);
        let alive = Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(!alive, "the helper outlived its watcher");
    }

    #[test]
    fn the_script_names_what_it_watches() {
        let s = script(&["com.spotify.client.PlaybackStateChanged"]);
        assert!(s.contains("[\"com.spotify.client.PlaybackStateChanged\"]"));
        assert!(s.contains("$.exit(0)"), "ends when stdin closes");
        assert!(s.contains("getppid() == parent"), "or its parent goes");
    }
}
