"""Nautilus extension: "Send to phone" entry directly in
the right-click menu (not under "Scripts").

Installation:
    sudo apt install python3-nautilus
    mkdir -p ~/.local/share/nautilus-python/extensions
    cp send_to_phone.py ~/.local/share/nautilus-python/extensions/
    nautilus -q          # restart Files

Then: right-click on one or more file(s) → "Send to phone".
"""

import os
import subprocess

from gi.repository import GObject, Nautilus

LINKD = os.path.expanduser("~/.local/bin/linkd")


class SendToPhoneExtension(GObject.GObject, Nautilus.MenuProvider):
    def _send(self, menu, files):
        for f in files:
            if f.get_uri_scheme() != "file":
                continue
            path = f.get_location().get_path()
            if path and os.path.isfile(path):
                subprocess.Popen([LINKD, "send-file", path])

    def get_file_items(self, *args):
        # The signature changed depending on the nautilus-python version:
        # (window, files) on older ones, (files,) on recent ones.
        files = args[-1]
        if not files:
            return []
        # Only show if at least one real file is selected.
        if not any(f.get_uri_scheme() == "file" for f in files):
            return []

        item = Nautilus.MenuItem(
            name="LinuxLink::send_to_phone",
            label="Send to phone",
            tip="Send the selection to the phone via Linux Link",
        )
        item.connect("activate", self._send, files)
        return [item]
