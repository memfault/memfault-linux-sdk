FROM ubuntu:latest

ENV DEBIAN_FRONTEND noninteractive

RUN apt-get update && apt-get -y upgrade

# Core utilities
RUN apt-get install -y bash build-essential curl git sudo vim

# Yocto dependencies
RUN apt-get install -y chrpath cpio diffstat gawk file lz4 python3-distutils wget zstd

# Yocto QEMU dependencies
RUN apt-get install -y samba

# Testing dependencies
RUN apt-get install -y python3-pytest python3-pexpect

# CppUnit testing dependencies for memfaultd's unit tests
RUN apt-get install -y \
    cmake \
    libcpputest-dev \
    libcurl4-openssl-dev \
    libjson-c-dev \
    libsystemd-dev \
    libubootenv-dev \
    pkg-config

# Add normal user
RUN useradd --create-home --gid users --groups sudo --shell /bin/bash build
RUN sed -i -e 's/^\(%sudo[[:space:]].*\)ALL$/\1NOPASSWD: ALL/' /etc/sudoers

# Add en_US.UTF-8 locale, required by Yocto
RUN apt-get install -y locales
RUN sed -i -e 's/^# en_US.UTF-8 UTF-8$/en_US.UTF-8 UTF-8/' /etc/locale.gen && echo 'LANG="en_US.UTF-8"' | tee -a /etc/default/locale && dpkg-reconfigure locales && update-locale LANG=en_US.UTF-8

USER build
WORKDIR /home/build

# Setup volume mounts
RUN mkdir -p /home/build/yocto/build
VOLUME /home/build/yocto/build
RUN mkdir -p /home/build/yocto/sources
VOLUME /home/build/yocto/sources

EXPOSE 5900

COPY entrypoint.sh /
COPY test.sh /
ENTRYPOINT ["/entrypoint.sh"]
CMD ["/bin/bash"]
