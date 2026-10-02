#!/usr/bin/env python3
"""A fake MPRIS player on the D-Bus session bus, for testing LavaTUI on Linux.

Implements org.mpris.MediaPlayer2 and org.mpris.MediaPlayer2.Player over a
small made-up playlist (no real music, no real album art), and changes its
state for every method and writable property the way a real player does.

    fake_mpris.py                      # org.mpris.MediaPlayer2.fakeplayer
    fake_mpris.py --name vlc           # another bus name
    fake_mpris.py --spotify            # as Spotify on Linux behaves
    fake_mpris.py --no-trackid         # no mpris:trackid, as some browsers

--spotify takes the name org.mpris.MediaPlayer2.spotify and copies the
Spotify client's known quirks: Position always reads 0, and setting
Shuffle / LoopStatus is accepted but does nothing.

Every call is printed to stdout as one line ("call PlayPause",
"set Volume 0.42", ...), flushed, so a test can follow along. The player
quits on SIGTERM / SIGINT or the MediaPlayer2.Quit method.

Needs python3-dbus-next (Debian/Ubuntu: apt install python3-dbus-next).
"""

import argparse
import asyncio
import signal
import sys
import time

from dbus_next import PropertyAccess, Variant
from dbus_next.aio import MessageBus
from dbus_next.service import ServiceInterface, dbus_property, method, signal as dbus_signal

PATH = "/org/mpris/MediaPlayer2"
NO_TRACK = "/org/mpris/MediaPlayer2/TrackList/NoTrack"

# Made-up tracks: (id part, title, artists, album, seconds).
TRACKS = [
    ("t0", "Slow Bloom", ["The Wax Hearts"], "Lamp Light", 241),
    ("t1", "Convection", ["Mara Vell", "Ode Kiri"], "Lamp Light", 198),
    ("t2", "Paraffin Sky", ["The Wax Hearts"], "Night Tank", 305),
]


def log(line):
    print(line, flush=True)


class Player:
    """The player's state, shared by both interfaces."""

    def __init__(self, spotify, trackid=True):
        self.spotify = spotify
        self.trackid = trackid
        self.index = 0
        self.status = "Playing"
        self.loop = "None"
        self.shuffle = False
        self.volume = 0.5
        # Position: offset at `since` (monotonic), moving while playing.
        self.offset_us = 12_000_000
        self.since = time.monotonic()
        self.opened_uri = None

    def track_id(self):
        part = TRACKS[self.index][0]
        if self.spotify:
            return f"/com/spotify/track/fake{part}"
        return f"/org/lavatui/fake/track/{part}"

    def metadata(self):
        part, title, artists, album, seconds = TRACKS[self.index]
        if self.opened_uri is not None:
            title = f"Opened {self.opened_uri}"
        art = (
            f"https://i.scdn.co/image/lavatui-fake-{part}"
            if self.spotify
            else f"file:///nonexistent/lavatui-fake-{part}.png"
        )
        metadata = {
            "mpris:length": Variant("x", seconds * 1_000_000),
            "mpris:artUrl": Variant("s", art),
            "xesam:title": Variant("s", title),
            "xesam:artist": Variant("as", artists),
            "xesam:album": Variant("s", album),
            "xesam:trackNumber": Variant("i", self.index + 1),
        }
        if self.trackid:
            metadata["mpris:trackid"] = Variant("o", self.track_id())
        return metadata

    def length_us(self):
        return TRACKS[self.index][4] * 1_000_000

    def position_us(self):
        pos = self.offset_us
        if self.status == "Playing":
            pos += int((time.monotonic() - self.since) * 1_000_000)
        return max(0, min(pos, self.length_us()))

    def set_position(self, us):
        self.offset_us = max(0, min(us, self.length_us()))
        self.since = time.monotonic()

    def set_status(self, status):
        self.set_position(self.position_us())  # freeze / restart the clock
        self.status = status

    def go(self, step):
        self.index = (self.index + step) % len(TRACKS)
        self.opened_uri = None
        self.set_position(0)


class Root(ServiceInterface):
    def __init__(self, player, quit_event):
        super().__init__("org.mpris.MediaPlayer2")
        self.player = player
        self.quit_event = quit_event

    @method()
    def Raise(self):
        log("call Raise")

    @method()
    def Quit(self):
        log("call Quit")
        self.quit_event.set()

    @dbus_property(access=PropertyAccess.READ)
    def CanQuit(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanRaise(self) -> "b":
        return False

    @dbus_property(access=PropertyAccess.READ)
    def HasTrackList(self) -> "b":
        return False

    @dbus_property(access=PropertyAccess.READ)
    def Identity(self) -> "s":
        return "Spotify" if self.player.spotify else "Fake Player"

    @dbus_property(access=PropertyAccess.READ)
    def DesktopEntry(self) -> "s":
        return "spotify" if self.player.spotify else "fakeplayer"

    @dbus_property(access=PropertyAccess.READ)
    def SupportedUriSchemes(self) -> "as":
        return ["spotify"] if self.player.spotify else ["file", "fake"]

    @dbus_property(access=PropertyAccess.READ)
    def SupportedMimeTypes(self) -> "as":
        return []


class PlayerIface(ServiceInterface):
    def __init__(self, player):
        super().__init__("org.mpris.MediaPlayer2.Player")
        self.p = player

    def changed(self, *names):
        values = {
            "PlaybackStatus": lambda: self.p.status,
            "Metadata": self.p.metadata,
            "LoopStatus": lambda: self.p.loop,
            "Shuffle": lambda: self.p.shuffle,
            "Volume": lambda: self.p.volume,
        }
        self.emit_properties_changed({n: values[n]() for n in names})

    # Methods.

    @method()
    def Next(self):
        log("call Next")
        self.p.go(1)
        self.changed("Metadata")

    @method()
    def Previous(self):
        log("call Previous")
        self.p.go(-1)
        self.changed("Metadata")

    @method()
    def Pause(self):
        log("call Pause")
        self.p.set_status("Paused")
        self.changed("PlaybackStatus")

    @method()
    def Play(self):
        log("call Play")
        self.p.set_status("Playing")
        self.changed("PlaybackStatus")

    @method()
    def PlayPause(self):
        log("call PlayPause")
        self.p.set_status("Paused" if self.p.status == "Playing" else "Playing")
        self.changed("PlaybackStatus")

    @method()
    def Stop(self):
        log("call Stop")
        self.p.set_status("Stopped")
        self.p.set_position(0)
        self.changed("PlaybackStatus")

    @method()
    def Seek(self, offset: "x"):
        log(f"call Seek {offset}")
        self.p.set_position(self.p.position_us() + offset)
        self.Seeked(self.p.position_us())

    @method()
    def SetPosition(self, track_id: "o", position: "x"):
        log(f"call SetPosition {track_id} {position}")
        # The spec: ignore a stale track id or a position past the end.
        if track_id != self.p.track_id() or not 0 <= position <= self.p.length_us():
            log("ignored SetPosition")
            return
        self.p.set_position(position)
        self.Seeked(position)

    @method()
    def OpenUri(self, uri: "s"):
        log(f"call OpenUri {uri}")
        self.p.opened_uri = uri
        self.p.set_position(0)
        self.p.status = "Playing"
        self.changed("Metadata", "PlaybackStatus")

    @dbus_signal()
    def Seeked(self, position) -> "x":
        return position

    # Properties.

    @dbus_property(access=PropertyAccess.READ)
    def PlaybackStatus(self) -> "s":
        return self.p.status

    @dbus_property()
    def LoopStatus(self) -> "s":
        return self.p.loop

    @LoopStatus.setter
    def LoopStatus(self, value: "s"):
        log(f"set LoopStatus {value}")
        if self.p.spotify:
            return  # Spotify on Linux: accepted, ignored.
        if value in ("None", "Track", "Playlist"):
            self.p.loop = value
            self.changed("LoopStatus")

    @dbus_property()
    def Rate(self) -> "d":
        return 1.0

    @Rate.setter
    def Rate(self, value: "d"):
        log(f"set Rate {value}")

    @dbus_property()
    def Shuffle(self) -> "b":
        return self.p.shuffle

    @Shuffle.setter
    def Shuffle(self, value: "b"):
        log(f"set Shuffle {value}")
        if self.p.spotify:
            return  # Spotify on Linux: accepted, ignored.
        self.p.shuffle = value
        self.changed("Shuffle")

    @dbus_property(access=PropertyAccess.READ)
    def Metadata(self) -> "a{sv}":
        return self.p.metadata()

    @dbus_property()
    def Volume(self) -> "d":
        return self.p.volume

    @Volume.setter
    def Volume(self, value: "d"):
        log(f"set Volume {value}")
        self.p.volume = max(0.0, min(value, 1.0))
        self.changed("Volume")

    @dbus_property(access=PropertyAccess.READ)
    def Position(self) -> "x":
        # Spotify on Linux has long reported 0 here.
        return 0 if self.p.spotify else self.p.position_us()

    @dbus_property(access=PropertyAccess.READ)
    def MinimumRate(self) -> "d":
        return 1.0

    @dbus_property(access=PropertyAccess.READ)
    def MaximumRate(self) -> "d":
        return 1.0

    @dbus_property(access=PropertyAccess.READ)
    def CanGoNext(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanGoPrevious(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanPlay(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanPause(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanSeek(self) -> "b":
        return True

    @dbus_property(access=PropertyAccess.READ)
    def CanControl(self) -> "b":
        return True


async def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--name", default="fakeplayer", help="bus name suffix")
    parser.add_argument("--spotify", action="store_true", help="act as Spotify")
    parser.add_argument("--paused", action="store_true", help="start paused")
    parser.add_argument(
        "--no-trackid", action="store_true", help="leave out mpris:trackid (as some browsers do)"
    )
    args = parser.parse_args()
    name = "spotify" if args.spotify else args.name

    player = Player(args.spotify, trackid=not args.no_trackid)
    if args.paused:
        player.status = "Paused"  # exactly at the start offset
    quit_event = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, quit_event.set)

    bus = await MessageBus().connect()
    bus.export(PATH, Root(player, quit_event))
    bus.export(PATH, PlayerIface(player))
    await bus.request_name(f"org.mpris.MediaPlayer2.{name}")
    log(f"ready org.mpris.MediaPlayer2.{name}")
    await quit_event.wait()
    bus.disconnect()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        sys.exit(0)
