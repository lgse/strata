#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
cat > /etc/vsftpd/vsftpd.conf <<CONF
listen=YES
background=NO
anonymous_enable=NO
local_enable=YES
write_enable=YES
chroot_local_user=YES
allow_writeable_chroot=YES
seccomp_sandbox=NO
isolate=NO
isolate_network=NO
pasv_enable=YES
pasv_address=127.0.0.1
pasv_min_port=${PASV_MIN_PORT}
pasv_max_port=${PASV_MAX_PORT}
ssl_enable=YES
allow_anon_ssl=NO
force_local_logins_ssl=NO
force_local_data_ssl=NO
require_ssl_reuse=NO
ssl_ciphers=HIGH
rsa_cert_file=/etc/ssl/certs/vsftpd.crt
rsa_private_key_file=/etc/ssl/private/vsftpd.key
CONF
exec /usr/sbin/vsftpd /etc/vsftpd/vsftpd.conf
