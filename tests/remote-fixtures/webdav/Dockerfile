# SPDX-License-Identifier: MIT
# Disposable Apache WebDAV server over HTTP and HTTPS with a self-signed
# certificate and Basic authentication. Never expose it beyond loopback.
FROM docker.io/library/alpine:3.22
RUN apk add --no-cache apache2 apache2-webdav apache2-ssl apache2-utils openssl \
 && rm -f /etc/apache2/conf.d/ssl.conf \
 && mkdir -p /srv/dav/files /var/lib/dav \
 && echo 'hello from the WebDAV fixture' > /srv/dav/files/hello-dav.txt \
 && chown -R apache:apache /srv/dav /var/lib/dav \
 && htpasswd -bc /etc/apache2/dav.passwd davuser fixture-password \
 && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj /CN=strata-fixture \
      -keyout /etc/ssl/apache2/fixture.key -out /etc/ssl/apache2/fixture.crt
COPY strata-dav.conf /etc/apache2/conf.d/strata-dav.conf
EXPOSE 80 443
CMD ["httpd", "-DFOREGROUND"]
