# SPDX-License-Identifier: MIT
# Disposable vsftpd server offering plaintext FTP and explicit FTPS (AUTH TLS)
# with a self-signed certificate. Never expose it beyond loopback.
FROM docker.io/library/alpine:3.22
RUN apk add --no-cache vsftpd openssl \
 && mkdir -p /srv/ftp \
 && adduser -D -h /srv/ftp/ftpuser ftpuser \
 && echo 'ftpuser:fixture-password' | chpasswd \
 && echo 'hello from the FTP fixture' > /srv/ftp/ftpuser/hello-ftp.txt \
 && chown -R ftpuser:ftpuser /srv/ftp/ftpuser \
 && mkdir -p /etc/ssl/private \
 && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=strata-fixture \
      -keyout /etc/ssl/private/vsftpd.key -out /etc/ssl/certs/vsftpd.crt
COPY entrypoint.sh /entrypoint.sh
EXPOSE 21
ENTRYPOINT ["/bin/sh", "/entrypoint.sh"]
