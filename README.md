# Dotfiles Manager

A small program to manage my dotfiles and keep them in sync between all my computers.
- must be _very_ simple to install and deploy:
    - Shipped as a single static binary (installable anywhere by a simple `curl`)
    - Small-ish binary (less than 2MB)
    - single command to configure the program (optional, can also use environment variables)
    - single command to pull the files on a new computer
    - single command to add files/sync to/from the remote
- Storage engine: S3. _Ideally from a public bucket so that no credentials are required to fetch files_. Authentication is however possible (at least for uploads)
- Versioning is outside of the scope of this tool (can be handled by S3)
- Basic conflict reconciliation by printing diffs
- CI pipeline to autogenerate windows / mac / linux binaries
- Designed for a moderate (<100) number of small (<1MB) text files.

**It is currently a beta that I built to define the APIs and figure out how to deal with the corner cases**.
It has all the basic features. I plan to use it for a while and polish it.

Potential improvements:
- Use async to handle more files (will currently check them sequentially)
- See if I can avoid downloading the files to check for diffs (remote md5? manifest)
- Support non-text files
- Unit tests
- Implement a proper hierarchy for parameters (cli args / config file / env variable). It is only partially implemented for now
- use different return codes
- Improve the logs, the level/source doesn't matter outside of --verbose
- Implement ignore, and give the option to add to the ignore list during sync
## Examples

### Fetch config on a new computer (no config, public S3 bucket, raspberry pi)
```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.2.3/dotfile.aarch64 -o /tmp/dotfile
$ chmod +x /tmp/dotfile
$ DOT_REMOTE=dotfile-bucket /tmp/dotfile sync
INFO  [dotfile] Listing files from dotfile-bucket
INFO  [dotfile]     Identical content, skipping: .config/fish/config.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/c.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/ff.fish
...
```

### Sync on a new computer (with config, private S3 bucket using the aws profile `aws_profile`)

```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.2.3/dotfile -o ~/bin/dotfile
$ chmod +x ~/bin/dotfile
$ dotfile configure -p aws_profile dotfile-bucket us-east-1
INFO  [dotfile] New configuration saved in /home/simon/.dots
$ dotfile sync
INFO  [dotfile] Listing files from dotfile-bucket
INFO  [dotfile]     Identical content, skipping: .config/fish/config.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/c.fish
INFO  [dotfile]     Identical content, skipping: .config/fish/functions/ff.fish
...
```

NOTE: The environment variables `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` can be provided during a sync instead of selecting a profile a config time.

### Track/forget a file (require authentication)
```
$ curl -L https://github.com/Blizarre/dotfiles_manager/releases/download/0.2.3/dotfile -o ~/bin/dotfile
$ chmod +x ~/bin/dotfile
$ dotfile configure -p aws_profile dotfile-bucket us-east-1
INFO  [dotfile] New configuration saved in /home/simon/.dots
$ dotfile track ~/.bashrc
INFO  [dotfile] Uploading /home/simon/.bashrc to .bashrc
$ dotfile forget ~/.bashrc
INFO  [dotfile] The file .bashrc has been removed
```