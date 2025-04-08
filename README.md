# journald-desktop-notifier

System journal error notifier

## Configuration

Specify matches and their edge cases. Make sure to include a `PRIORITY` match in your configuration.

```toml
[[match]]
# 0 = emerg, 1 = alert, 2 = critical, 3 = error
PRIORITY = "^0|1|2|3$"

# It's fully recursive
[[match]]
SYSLOG_IDENTIFIER = "^foo$"
MESSAGE = "^bar"
[[match.__allow]]
MESSAGE = "^bar spam"
[[match.__allow.__deny]]
_SYSTEMD_UNIT = "^but-actually.service$"
```

The example above translates to the following boolean condition:
```
PRIORITY ~= "^0|1|2|3$"
|| (
    SYSLOG_IDENTIFIER ~= "^foo$"
    && MESSAGE ~= "^bar"
    && !(
        MESSAGE ~= "^bar spam"
        && !(_SYSTEMD_UNIT ~= "^but-actually.service$")
    )
)
```

## Development

0. Have Linux

1. Install [Nix](https://nixos.org/download#download-nix)

2. Run the command `nix develop` in a shell.

   This creates a `bash` subshell with all the dependencies.

3. Run `cargo` commands as you like.

   i.e. `cargo build`, `cargo run`, `cargo clippy`, etc.

## Contributing

Please first make sure that you have not introduced any regressions and format the code by running the following commands at the repository root.
```sh
cargo fmt
cargo clippy
cargo test
```

Make a GitHub [pull request](https://github.com/axelkar/journald-desktop-notifier/pulls).

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
