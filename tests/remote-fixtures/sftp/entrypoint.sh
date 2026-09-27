#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
for user in keyed locked; do
  install -d -m 700 -o "$user" -g "$user" "/home/$user/.ssh"
  install -m 600 -o "$user" -g "$user" "/fixture/$user.pub" "/home/$user/.ssh/authorized_keys"
done
exec /usr/sbin/sshd -D -e
