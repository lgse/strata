# SPDX-License-Identifier: MIT
"""Reads the kernel log through /dev/kmsg, as dmesg does.

kmsg.py last                prints the sequence number of the newest record
kmsg.py since SEQ SERVER    prints the cifs messages naming the UNC server SERVER
                            logged after record SEQ
"""
import os
import re
import sys


def records():
    fd = os.open("/dev/kmsg", os.O_RDONLY | os.O_NONBLOCK)
    try:
        while True:
            try:
                record = os.read(fd, 8192)
            except BlockingIOError:
                return
            except BrokenPipeError:
                # Records were overwritten while reading; the next read resumes.
                continue
            head, _, text = record.decode(errors="replace").partition(";")
            # kmsg escapes backslashes, so UNC names arrive as \x5c\x5cserver\x5cshare.
            yield int(head.split(",")[1]), text.split("\n", 1)[0].replace("\\x5c", "\\")
    finally:
        os.close(fd)


def names_server(text, server):
    """The kernel log is the host's; only lines naming the lab's server are the lab's."""
    return re.search(rf"(\\\\|//){re.escape(server)}(?![\w.-])", text) is not None


def main():
    if sys.argv[1:] == ["last"]:
        print(max((seq for seq, _ in records()), default=-1))
    elif len(sys.argv) == 4 and sys.argv[1] == "since":
        start, server = int(sys.argv[2]), sys.argv[3]
        for seq, text in records():
            if seq > start and "cifs" in text.lower() and names_server(text, server):
                print(text)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
