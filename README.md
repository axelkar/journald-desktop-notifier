# journald-desktop-notifier

System journal error notifier

## Configuration

Configuration can be written in either TOML or JSON. JSON support has been included to enable writing configuration automatically. Specify matches and their edge cases. Make sure to include a `PRIORITY` match in your configuration. View the possible fields using `journalctl -b0 -o verbose`.

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

## Home Manager module

This project provides a [Home Manager](https://github.com/nix-community/home-manager/) module. To use it, write something like the following into your configuration.

```nix
{ ... }:
{
    imports = [ inputs.journald-desktop-notifier.homeModules.default ];
    services.journald-desktop-notifier = {
        enable = true;
        settings.match = [{
            PRIORITY = "^0|1|2|3$";
            __allow = [{
                SYSLOG_IDENTIFIER = "^systemd-coredump$";
                MESSAGE = "user 30001"; # nixbld1
            }];
        ]};
    };
}
```

## NixOS module

This project also provides a NixOS module, providing similar functionality and
configuration options. Note that it defines user units. Here's an example:

```nix
{ ... }:
{
    imports = [ inputs.journald-desktop-notifier.nixosModules.default ];
    services.journald-desktop-notifier = {
        enable = true;
        settings.match = [{
            PRIORITY = "^0|1|2|3$";
            __allow = [{
                SYSLOG_IDENTIFIER = "^systemd-coredump$";
                MESSAGE = "user 30001"; # nixbld1
            }];
        ]};
    };
}
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
