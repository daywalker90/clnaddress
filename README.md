[![latest release on CLN v25.02](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v25.02.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v25.02.yml) [![latest release on CLN v24.11](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v24.11.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v24.11.yml) [![latest release on CLN v24.08.2](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v24.08.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/latest_v24.08.yml)

[![main on CLN v25.02](https://github.com/daywalker90/clnaddress/actions/workflows/main_v25.02.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/main_v25.02.yml) [![main on CLN v24.11](https://github.com/daywalker90/clnaddress/actions/workflows/main_v24.11.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/main_v24.11.yml) [![main on CLN v24.08.2](https://github.com/daywalker90/clnaddress/actions/workflows/main_v24.08.yml/badge.svg?branch=main)](https://github.com/daywalker90/clnaddress/actions/workflows/main_v24.08.yml)

# clnaddress
A core lightning plugin that runs an lnurl server to support lnurl and ln-addresses with optional Zap support.

:white_check_mark: [LUD-06](https://github.com/lnurl/luds/blob/luds/06.md)

:white_check_mark: [LUD-16](https://github.com/lnurl/luds/blob/luds/16.md)

:white_check_mark: [NIP-57](https://github.com/nostr-protocol/nips/blob/master/57.md)


* [Installation](#installation)
* [Building](#building)
* [Options](#options)
* [Methods](#methods)
* [Documentation](#documentation)

# Installation
For general plugin installation instructions see the plugins repo [README.md](https://github.com/lightningd/plugins/blob/master/README.md#Installation)

Release binaries for
* x86_64-linux
* armv7-linux (Raspberry Pi 32bit)
* aarch64-linux (Raspberry Pi 64bit)

can be found on the [release](https://github.com/daywalker90/clnaddress/releases) page. If you are unsure about your architecture you can run ``uname -m``.

They require ``glibc>=2.31``, which you can check with ``ldd --version``.

# Building
You can build the plugin yourself instead of using the release binaries.
First clone the repo:

```
git clone https://github.com/daywalker90/clnaddress.git
```

Install a recent rust version ([rustup](https://rustup.rs/) is recommended) and in the ``clnaddress`` folder run:

```
cargo build --release
```

After that the binary will be here: ``target/release/clnaddress``

Note: Release binaries are built using ``cross`` and the ``optimized`` profile.


# Options
- ``clnaddress-min-receivable``: Minimum receivable amount in msat, defaults to ``1``
- ``clnaddress-max-receivable``: Maximum receivable amount in msat, defaults to ``100000000000``
- ``clnaddress-description``: Description shown in wallets, defaults to ``Thank you :)``
- ``clnaddress-listen``: Listen address for the LNURL web server. Use ``[::]`` to bind to everything. Defaults to ``localhost:9797``
- ``clnaddress-base-url``: Base URL of you lnaddress service, e.g. ``https://sub.domain.org/path/``, no default and must be set
- ``clnaddress-nostr-privkey``: Nostr private key for signing zap receipts, no default and optional, but required for zap support

# Methods
* **clnaddress-adduser** *user* [*is_email*] [*description*]
     * adds a user for your ln-address server
     * ***user***: username part of the lightning address
     * ***is_email***: optional boolean if the lightning address is also an email, which would change the metadata slightly, defaults to ``false``
     * ***description***: optional user-specific description, defaults to the description from the ``clnaddress-description`` option
* **clnaddress-deluser** *user*
     * deletes a previously added user
     * ***user***: username part of the lightning address

# Documentation

### Reverse Proxy
For any of this to work you must configure your reverse proxy to point to the lnurl web server hosted under ``clnaddress-listen``

With ``nginx``:

For LNURL you can choose any location e.g. ``lnurl``:
```
location /lnurl/ {
        proxy_pass http://localhost:9797/;
        add_header 'Access-Control-Allow-Origin' '*';
}
```
To support all ln-address users you add another location like this:
```
location ~* ^/\.well-known/lnurlp/([^/]+) {
        proxy_pass http://localhost:9797$request_uri;
        add_header 'Access-Control-Allow-Origin' '*';
}
```
Make sure to use the correct ``proxy_pass`` address, usually it's just ``http://`` + ``clnaddress-listen`` + ``/``

### LNURL
Your LNURL gets printed to log on plugin start, watch out for the line starting with ``LNURL:``

### LN-Addresses
``clnaddress`` supports multiple ln-addresses at the same time and you can add or remove users with the ``clnaddress-adduser`` and ``clnaddress-deluser`` methods.

### Nostr
In order for zap receipts to be send you must specify a ``clnaddress-nostr-privkey`` that will sign the receipts. It is recommended to create another key for this and not use your usual nostr key.


