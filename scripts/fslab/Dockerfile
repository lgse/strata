# check=skip=InvalidDefaultArgInFrom
# Built by scripts/fslab/fslab.sh on the verified test image.
ARG BASE
FROM ${BASE}

# exfat-utils (exfat-fuse) conflicts with exfatprogs, so only its mount helper is
# extracted from the signature-verified package.
RUN sh /opt/strata-install-packages.sh indexes \
 && sh /opt/strata-install-packages.sh install \
      nfs-utils rpcbind samba cifs-utils dosfstools exfatprogs ntfs-3g ntfsprogs \
      e2fsprogs python kmod fuse3 procps-ng strace sshfs openssh socat \
 && pacman -Sw --noconfirm exfat-utils \
 && bsdtar -xf /var/cache/pacman/pkg/exfat-utils-*.pkg.tar.zst -C / usr/bin/mount.exfat-fuse \
 && /usr/bin/mount.exfat-fuse -V \
 && pacman -Scc --noconfirm \
 && rm -rf /var/lib/pacman/sync/* /var/cache/pacman/pkg/*

ARG FSLAB_UID
ARG FSLAB_GID
RUN groupadd -o -g "${FSLAB_GID}" strata \
 && useradd -o -u "${FSLAB_UID}" -g "${FSLAB_GID}" -m -d /home/strata -s /bin/bash strata \
 && install -d -o strata -g strata /cache/cargo /cache/target

WORKDIR /
CMD ["sleep", "infinity"]
