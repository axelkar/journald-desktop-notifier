nixosModule: {
  name = "journald-desktop-notifier";

  nodes.machine =
    { pkgs, ... }:
    let
      mockPython = pkgs.python3.withPackages (ps: [ ps.python-dbusmock ]);
    in
    {
      imports = [ nixosModule ];

      users.users.alice = {
        isNormalUser = true;
        # Journal files are readable by wheel and adm through ACLs or by the
        # systemd-journal group that owns them.
        extraGroups = [ "systemd-journal" ];
      };

      services.journald-desktop-notifier = {
        enable = true;
        # Nothing starts graphical-session.target in a headless VM and it
        # refuses manual starts. default.target is started by the user manager
        # and exercises the same After=/WantedBy= mechanism.
        systemdTarget = "default.target";
        settings.match = [
          {
            PRIORITY = "^0|1|2|3$";
            ignore = [ { MESSAGE = "^ignore me"; } ];
          }
        ];
      };

      systemd.user.services.notify-mock = {
        description = "Mock org.freedesktop.Notifications service";
        wantedBy = [ "default.target" ];
        after = [ "dbus.socket" ];
        serviceConfig = {
          Type = "dbus"; # Active only once the name is acquired.
          BusName = "org.freedesktop.Notifications";
          Restart = "on-failure";
          ExecStart = "${mockPython}/bin/python -m dbusmock --template notification_daemon --logfile /tmp/notifications";
        };
      };
    };

  testScript = ''
    start_all()

    # Start the user manager.
    uid = machine.succeed("id -u alice").strip()
    machine.systemctl(f"start user@{uid}.service")
    machine.wait_for_unit(f"user@{uid}.service")

    # A target is only reached after the units it wants have started, so the
    # mock owns the bus name by the time default.target is active.
    machine.wait_for_unit("default.target", user="alice")
    machine.wait_for_unit("notify-mock.service", user="alice")
    machine.wait_for_unit("journald-desktop-notifier.service", user="alice")

    with subtest("matching entry notifies"):
        machine.succeed("logger -p daemon.err -t test-app test-notification-1")
        machine.wait_until_succeeds("grep -q 'test-notification-1' /tmp/notifications")
        machine.succeed("grep -q 'Notify \"test-app\"' /tmp/notifications")

    # The matching message acts as a barrier: entries are processed in order,
    # so once its notification is logged the ignored entry was processed too.
    with subtest("ignored entry does not notify"):
        machine.succeed("logger -p daemon.err -t test-app 'ignore me please'")
        machine.succeed("logger -p daemon.err -t test-app test-notification-2")
        machine.wait_until_succeeds("grep -q test-notification-2 /tmp/notifications")
        machine.fail("grep -q 'ignore me please' /tmp/notifications")

    with subtest("non-matching priority does not notify"):
        machine.succeed("logger -p daemon.info -t test-app test-info")
        machine.succeed("logger -p daemon.err -t test-app test-notification-3")
        machine.wait_until_succeeds("grep -q test-notification-3 /tmp/notifications")
        machine.fail("grep -q test-info /tmp/notifications")
  '';
}
