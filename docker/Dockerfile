FROM ubuntu:latest

# **NOTE: DO NOT PUT ANYTHING SECRET / CONFIDENTIAL IN THE DOCKER IMAGE** The
# image is publically published.

ENV DEBIAN_FRONTEND noninteractive
ENV YOCTO_RELEASE ${YOCTO_RELEASE}

RUN dpkg --add-architecture i386 && \
    apt-get update && \
    apt-get -y upgrade

# Core utilities
RUN apt-get install -y bash build-essential curl git sudo vim

# Yocto dependencies
RUN apt-get install -y chrpath cpio diffstat gawk file lz4 python3-distutils wget zstd

# Yocto QEMU dependencies
RUN apt-get install -y samba

# Yocto wic dependencies
RUN apt-get install -y parted

# Install pip for python testing dependencies
RUN apt-get install -y python3-pip

# CppUnit testing dependencies for memfaultd's unit tests
RUN apt-get install -y \
    cmake \
    gcc-multilib \
    g++-multilib \
    libcurl4-openssl-dev \
    libjson-c-dev \
    libjson-c-dev:i386 \
    libsystemd-dev \
    libsystemd-dev:i386 \
    libubootenv-dev \
    uuid-dev \
    uuid-dev:i386 \
    pkg-config

# Work-around for conflicting binary (32 vs 64 bit) -- we don't use/need it
RUN rm -rf /usr/bin/curl-config && \
    apt-get install -y libcurl4-openssl-dev:i386

# Work-around for libubootenv-dev:i386 package not existing -- we only need the header to compiler the unit tests:
RUN cp /usr/include/x86_64-linux-gnu/libuboot.h /usr/include/i386-linux-gnu/libuboot.h

# CppUnit:
COPY install-cpputest.sh /
RUN /install-cpputest.sh && rm -rf /install-cpputest.sh

# Add normal user
RUN useradd --create-home --gid users --groups sudo --shell /bin/bash build
RUN sed -i -e 's/^\(%sudo[[:space:]].*\)ALL$/\1NOPASSWD: ALL/' /etc/sudoers

# Add en_US.UTF-8 locale, required by Yocto
RUN apt-get install -y locales
RUN sed -i -e 's/^# en_US.UTF-8 UTF-8$/en_US.UTF-8 UTF-8/' /etc/locale.gen && echo 'LANG="en_US.UTF-8"' | tee -a /etc/default/locale && dpkg-reconfigure locales && update-locale LANG=en_US.UTF-8

USER build
WORKDIR /home/build

RUN echo " \
    alias q='/home/build/yocto/sources/memfault-linux-sdk/test-scripts/runqemu.py'\n\
    alias b='bitbake memfault-image'\n\
    alias cc='rm -rf /home/build/yocto/build/conf'\n\
    alias ct='rm -rf /home/build/yocto/build/tmp'\n\
    alias build-memfault-cli-local='pip3 install /home/build/memfault-cli-dist/memfault-cli-*.tar.gz'\n\
" >> /home/build/.bashrc

# Install python testing dependencies
COPY requirements.txt /home/
RUN echo 'PATH=$HOME/.local/bin:$PATH' >> $HOME/.bashrc
RUN pip3 install --no-warn-script-location -r /home/requirements.txt

# Setup volume mounts
RUN mkdir -p /home/build/yocto/build
VOLUME /home/build/yocto/build
RUN mkdir -p /home/build/yocto/sources
VOLUME /home/build/yocto/sources
VOLUME /home/build/memfault-cli-dist

EXPOSE 5900

COPY entrypoint.sh /
COPY test.sh /
ENTRYPOINT ["/entrypoint.sh"]
CMD ["/bin/bash"]
