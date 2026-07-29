"""Nautilus extension: a "Send to phone" entry directly in the right-click
menu, rather than buried under "Scripts".

When several phones are connected the entry becomes a submenu, one line per
device, so the file goes where the user meant it to go. With a single phone it
stays a single click.

Installation:
    sudo apt install python3-nautilus
    mkdir -p ~/.local/share/nautilus-python/extensions
    cp send_to_phone.py ~/.local/share/nautilus-python/extensions/
    nautilus -q          # restart Files

Then: right-click on one or more file(s) -> "Send to phone".
"""

import json
import os
import subprocess

from gi.repository import GObject, Nautilus

LINKD = os.path.expanduser("~/.local/bin/linkd")
STATUS = os.path.join(
    os.environ.get("XDG_CONFIG_HOME", os.path.expanduser("~/.config")),
    "linux-link",
    "status.json",
)


def connected_devices():
    """The phones currently reachable, as (name, fingerprint) pairs.

    The daemon rewrites status.json whenever anything changes, so reading it is
    both cheap and current. Any failure here means "we do not know", and the
    menu falls back to the plain entry — a broken status file must never make
    "Send to phone" disappear.
    """
    try:
        with open(STATUS, "r", encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, ValueError):
        return []
    return [
        (d.get("name") or "Phone", d.get("fingerprint") or "")
        for d in data.get("devices", [])
        if d.get("connected")
    ]


def selected_paths(files):
    paths = []
    for f in files:
        if f.get_uri_scheme() != "file":
            continue
        path = f.get_location().get_path()
        if path and os.path.isfile(path):
            paths.append(path)
    return paths


class SendToPhoneExtension(GObject.GObject, Nautilus.MenuProvider):
    def _send(self, _menu, paths, fingerprint=None):
        for path in paths:
            cmd = [LINKD, "send-file", path]
            if fingerprint:
                cmd += ["--to", fingerprint]
            subprocess.Popen(cmd)

    def get_file_items(self, *args):
        # The signature changed with the nautilus-python version:
        # (window, files) on older ones, (files,) on recent ones.
        files = args[-1]
        if not files:
            return []
        paths = selected_paths(files)
        if not paths:
            return []

        devices = connected_devices()
        label = "Send to phone" if len(paths) == 1 else "Send %d files to phone" % len(paths)

        if len(devices) <= 1:
            item = Nautilus.MenuItem(
                name="LinuxLink::send_to_phone",
                label=label,
                tip="Send the selection to the phone via Linux Link",
            )
            fingerprint = devices[0][1] if devices else None
            item.connect("activate", self._send, paths, fingerprint)
            return [item]

        parent = Nautilus.MenuItem(
            name="LinuxLink::send_to_phone_menu",
            label=label,
            tip="Choose which device receives the selection",
        )
        submenu = Nautilus.Menu()
        for index, (name, fingerprint) in enumerate(devices):
            entry = Nautilus.MenuItem(
                name="LinuxLink::send_to_%d" % index,
                label=name,
                tip="Send to %s" % name,
            )
            entry.connect("activate", self._send, paths, fingerprint)
            submenu.append_item(entry)
        parent.set_submenu(submenu)
        return [parent]
