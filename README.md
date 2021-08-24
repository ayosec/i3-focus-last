i3-alternate-focus
==================

Fork from <https://github.com/lbonn/i3-focus-last>.

* The socket to send the *switch* command is now stored in a property of the root window, and it is stored in `$XDG_RUNTIME_DIR/i3-alternate-focus.$PID.$TIMESTAMP.sock`. Thus, we can have multiple instances of i3 in the same machine.
* Windows focused for less than 1 second are ignored.
* Focused windows is never discarded when `switch` command is invoked.
