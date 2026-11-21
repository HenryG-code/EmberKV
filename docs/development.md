# Development setup

How to get from a fresh machine to running EmberKV's code. Written for Windows 11 with WSL2, which is
how this project is developed; on native Ubuntu, skip the Windows parts.

## Where the code lives, and why

EmberKV is built and tested **inside Linux**, in WSL2 Ubuntu, even when the desktop is Windows.

| Location | Role |
|---|---|
| `~/projects/emberkv` inside Ubuntu | **The working copy.** Edit, build, test and commit here |
| GitHub | The source of truth and the backup. Push at the end of every session |
| Anything under `C:\` or `/mnt/c/` | **Not used** for EmberKV |

WSL2 keeps Ubuntu's files inside a virtual disk file on the Windows drive. Physically it is the same
disk, but it is a separate Linux filesystem (ext4). Ubuntu reaches the Windows drive through `/mnt/c/`,
and Windows reaches Ubuntu's files through `\\wsl.localhost\Ubuntu\`.

**Why not keep the code on the Windows drive:**

1. **Correctness.** EmberKV's tests are about what survives when data is flushed to a Linux disk.
   Files under `/mnt/c/` go through a translation layer to Windows' NTFS, which behaves differently,
   so crash and durability tests there would measure the translation layer, not EmberKV.
2. **Speed.** Rust builds read and write thousands of small files. Across `/mnt/c/` this is several
   times slower.
3. **Consistency.** CI runs on Ubuntu, and benchmarks run on native Ubuntu. Developing in Linux
   keeps all three the same.

**Rule of thumb:** if a path starts with `/mnt/c/` or `C:\`, you are in the wrong place.

## Telling where you are

| Prompt looks like | You are in | Use it for |
|---|---|---|
| `PS C:\...>` | Windows PowerShell | Starting Ubuntu, Windows tasks only |
| `user@PC:~/projects/emberkv$` | Ubuntu | Everything in this project |
| `docker-desktop:~#` | Docker Desktop's internal Linux | Nothing: type `exit` |

`pwd` prints the current folder. Paths starting with `~` or `/home/` are Ubuntu's own files.

## First-time setup

All in PowerShell first, then Ubuntu.

1. **Open Ubuntu by name.** Plain `wsl` may open Docker Desktop's internal system instead.

   ```powershell
   wsl -d Ubuntu
   ```

2. **Install the C build tools.** Rust uses the system linker and C library to produce Linux programs.

   ```bash
   sudo apt update
   sudo apt install build-essential
   ```

   `sudo` asks for the Ubuntu account's password, which is separate from Windows and GitHub. Nothing
   shows while typing it. If it is forgotten, reset it from PowerShell with
   `wsl -d Ubuntu -u root passwd <ubuntu-username>`.

3. **Install Rust** with `rustup`, the official installer, accepting the defaults:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source ~/.cargo/env
   rustc --version
   ```

4. **Configure Git inside Ubuntu.** Ubuntu's Git settings are separate from Windows'. Use GitHub's
   private no-reply address so a personal email never appears in public history (GitHub, Settings,
   Emails).

   ```bash
   git config --global user.name "Your Name"
   git config --global user.email "ID+username@users.noreply.github.com"
   ```

5. **Sign in to GitHub through Windows.** GitHub does not accept passwords for Git. If Git for Windows
   is installed, its Credential Manager can sign in through the browser for Ubuntu too:

   ```bash
   git config --global credential.helper '/mnt/c/Program\ Files/Git/mingw64/bin/git-credential-manager.exe'
   ```

6. **Clone into the Ubuntu home folder**, never under `/mnt/c/`:

   ```bash
   git clone https://github.com/HenryG-code/EmberKV.git ~/projects/emberkv
   ```

## Editing with VS Code

Install VS Code on Windows with the **WSL** extension, then open the project **from the Ubuntu
terminal**:

```bash
code ~/projects/emberkv
```

- The bottom-left corner must say **WSL: Ubuntu**. If it does not, the window is showing Windows
  files, and edits will not reach the code that Cargo builds.
- Install the **rust-analyzer** extension with "Install in WSL: Ubuntu". It shows types and errors
  while you type.
- VS Code's terminal (`` Ctrl+` ``) in that window already runs inside Ubuntu.
- **File, Open Recent** lists the project as `emberkv [WSL: Ubuntu]`.

To browse the files from Windows File Explorer, use `\\wsl.localhost\Ubuntu\home\<user>\projects\emberkv`.
Edit them only through VS Code in WSL mode.

## Daily workflow

```bash
cd ~/projects/emberkv
git switch main
git pull                          # get anything merged on GitHub
git switch -c type/short-name     # one branch per slice or change
# ... edit, cargo run, cargo test ...
git add -A
git commit -m "type: what and why"
git push -u origin type/short-name
```

Then open a pull request on GitHub and merge it. Never push straight to `main`.

## Common problems

| Symptom | Cause | Fix |
|---|---|---|
| `cp`, `cd` or `~` paths fail with `C:\...` in the error | The command ran in PowerShell | Start Ubuntu with `wsl -d Ubuntu` |
| `git: not found` with a `docker-desktop` prompt | Plain `wsl` opened Docker's system | `exit`, then `wsl -d Ubuntu` |
| `cannot open crti.o`, `unable to find library -lc` | C build tools missing | `sudo apt install build-essential` |
| `Password authentication is not supported` | GitHub no longer accepts passwords | Set the credential helper in step 5 |
| Program still runs old code, or a file is missing in the Explorer | VS Code is showing the Windows copy, or the file is unsaved | Reopen with `code ~/projects/emberkv`, check **WSL: Ubuntu**, save with `Ctrl+S` |
| `cd: No such file or directory` | A relative path from the wrong folder | Check `pwd`, or use a full path starting with `~` |
| `warning: unused dependency` after editing | The edit was not saved, so the code using it is not there | Save, then check with `head src/main.rs` |
