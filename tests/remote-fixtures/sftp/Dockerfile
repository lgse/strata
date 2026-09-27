# SPDX-License-Identifier: MIT
# Disposable OpenSSH server for scripts/remote-fixtures.py. Never expose it
# beyond loopback: the accounts use fixed test passwords.
FROM docker.io/library/alpine:3.22
RUN apk add --no-cache openssh-server openssh-sftp-server \
 && ssh-keygen -A \
 && for user in strata keyed locked; do adduser -D -s /bin/sh "$user"; done \
 && printf 'strata:fixture-password\nkeyed:%s\nlocked:%s\n' \
      "$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')" \
      "$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')" | chpasswd \
 && mkdir -p /srv/shared/nested \
 && echo 'hello from the SFTP fixture' > /srv/shared/hello.txt \
 && chmod -R a+rX /srv/shared \
 && printf '%s\n' 'PasswordAuthentication yes' 'KbdInteractiveAuthentication no' \
      'PermitRootLogin no' 'Subsystem sftp internal-sftp' >> /etc/ssh/sshd_config \
 && sed -i 's|^Subsystem[[:space:]]\+sftp[[:space:]]\+/.*||' /etc/ssh/sshd_config
COPY entrypoint.sh /entrypoint.sh
EXPOSE 22
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
