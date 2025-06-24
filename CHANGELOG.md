# Change Log

What changed in mapiproxy, per version


## mapiproxy NEXTVERSION - YYYY-MM-DD

- Add option `--brief[=N]` which shows only the first and last
  N lines of each block.


## mapiproxy 0.6.3 - 2025-01-10

- Make pcap parsing more permissive. Some traces 'in the wild' have incorrect
  length fields that don't matter much.


## mapiproxy 0.6.2 - 2024-04-25

- Add -o or --output= option to direct output to a file.

- Print timestamp marker before the first message of each minute.

- Support proxying [Out-Of-Band (OOB)][OOB] signals (Linux only).

[OOB]: https://en.wikipedia.org/wiki/Transmission_Control_Protocol#Out-of-band_data


## mapiproxy 0.6.1 - 2024-03-13

- Upgrade mio dependency, it had a security issue.
  Mapiproxy is not affected but we kept getting warnings.


## mapiproxy 0.6.0 - 2024-02-23

- No longer default to `--messages`. It's not clear what the default should
  be so for the time being it's better to not have a default at all.

- Add experimental `--pcap=FILE` option to read captured network traffic files
  written by for example, [tcpdump](https://www.tcpdump.org/).
  So far this has seen very limited testing.


## mapiproxy 0.5.2 - 2024-02-21

- Add option --color=always|never|auto to control the use of color escapes.
  'Auto' is 'on' on terminals, 'off' otherwise.

- Colorize text, digits and whitespace in binary output. This makes it easier
  to match the hex codes on the left to the characters on the right.

- Support raw IPv6 addresses in LISTEN_ADDR and FORWARD_ADDR, between square brackets.
  For example, `[::1]:50000`.

- Clean up Unix sockets when Control-C is pressed.


## mapiproxy 0.5.1 - 2024-02-16

- This release only exists because a version v0.5.1-alpha.1
  was uploaded to crates.io as an experiment.


## mapiproxy 0.5.0 - 2024-02-16

The basics work:

- Listen on TCP sockets and Unix Domain sockets

- Connect to TCP sockets and Unix Domain sockets

- Adjust the initial '0' (0x30) byte when proxying between Unix and TCP or vice
  versa

- Render either as raw reads and writes, full MAPI blocks or full MAPI messages

- Render as text or as a hex dump

- Pretty-print tabs and newlines

- In raw mode, highlight the MAPI block headers


## mapiproxy 0.3.0

Skipped due to experiments in another repo.


## mapiproxy 0.2.0

Skipped because predecessor 'monetproxy' was already at 0.2.0.


## mapiproxy 0.1.0 - unreleased

Initial version number picked by 'cargo new'
