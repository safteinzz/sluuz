#!/usr/bin/env bash
# The world the README pictures are taken in: six invented repositories, their
# invented history, and the leaked password planted in three of them. Nothing
# here touches your repos, your ~/.gitconfig or your git state - HOME and every
# XDG variable are redirected into ./home, which `down` deletes.
#
#   ./stage.sh up      build the fixtures
#   ./stage.sh shell   a shell where `slu` is this build and ~ is the stage
#   ./stage.sh down    delete the stage
#
# Every name is invented, every address is example.com (RFC 2606), every
# "secret" is the string 1337-let-me-in, and every remote is a bare repo under
# ./home/remotes whose URL is rewritten to a gitlab.com/acme one afterwards - so
# the ahead/behind counts on screen are real git tracking state, computed
# locally, with nothing to reach and nothing of yours to leak.
#
# There is no .env and nothing to configure: slu only ever reads git, so the
# whole demo is reproducible from this file alone.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAGE="$HERE/home"
WORK="$STAGE/work"
REMOTES="$STAGE/remotes"
SLU="$HERE/../target/release/slu"

# The leaked credential the search, scan and iscan shots are all about. It is a
# joke string, planted here, and it exists nowhere else.
SECRET="1337-let-me-in"

# ---------------------------------------------------------------------------
# the staged environment
# ---------------------------------------------------------------------------

# HOME alone is not enough: a shell that exports XDG_CONFIG_HOME would send git
# straight back to your real config. GIT_CEILING_DIRECTORIES matters as much:
# the stage sits inside sluuz's own checkout, so without a ceiling every prompt
# and every discovery walks out of the fixtures and reports on sluuz instead -
# which is how a screenshot of `~/work` ends up wearing this repo's branch.
stage_env() {
  echo "HOME=$STAGE" \
       "XDG_CONFIG_HOME=$STAGE/.config" \
       "XDG_DATA_HOME=$STAGE/.local/share" \
       "XDG_STATE_HOME=$STAGE/.local/state" \
       "XDG_CACHE_HOME=$STAGE/.cache" \
       "GIT_CONFIG_NOSYSTEM=1" \
       "GIT_CEILING_DIRECTORIES=$STAGE"
}

# Every git call in this script goes through here, so none of them can read or
# write the real home. GIT_CONFIG_NOSYSTEM drops /etc/gitconfig too, which is
# the one file a redirected HOME does not hide.
g() { env $(stage_env) git "$@"; }

# `c <repo> <seconds-ago> <who> <message>` - a commit with a chosen age, so the
# relative dates on screen ("3 days ago") tell a story instead of all reading
# "just now". Both dates are set: ilog shows the committer, tidy sorts on it.
c() {
  local repo=$1 age=$2 who=$3 msg=$4
  # A few hours of jitter, derived from the message so it is the same on every
  # render: without it every age is a whole number of days off one instant and
  # the time column in ilog reads as one repeated value.
  local jitter=$(( $(printf '%s' "$msg" | cksum | cut -d' ' -f1) % 28800 - 14400 ))
  local when="@$((NOW - age + jitter)) +0000"
  local name email
  case "$who" in
    ada)   name="Ada Weller";   email="ada@example.com" ;;
    marek) name="Marek Toth";   email="marek@example.com" ;;
    priya) name="Priya Nandan"; email="priya@example.com" ;;
    *)     name="$who";         email="$who@example.com" ;;
  esac
  g -C "$repo" add -A
  env $(stage_env) \
      GIT_AUTHOR_NAME="$name" GIT_AUTHOR_EMAIL="$email" GIT_AUTHOR_DATE="$when" \
      GIT_COMMITTER_NAME="$name" GIT_COMMITTER_EMAIL="$email" GIT_COMMITTER_DATE="$when" \
      git -C "$repo" commit -q -m "$msg"
}

# `origin <repo>` - a real bare remote under the stage, pushed to for real, so
# every ahead/behind count on screen is genuine git tracking state.
origin() {
  local repo=$1 name; name=$(basename "$repo")
  g init -q --bare "$REMOTES/$name.git"
  g -C "$repo" remote add origin "$REMOTES/$name.git"
  g -C "$repo" push -q -u origin --all
}

# Last thing `up` does: swap every origin URL for one that looks like a forge.
# The tracking refs stay exactly where they are - they are local refs - so the
# counts survive and the dashboard shows `gitlab.com:acme/<repo>` instead of a
# path out of this rig. Nothing is ever pushed or fetched after this point.
seal_remotes() {
  local repo name
  for repo in "$WORK"/*; do
    name=$(basename "$repo")
    g -C "$repo" config remote.origin.url "git@gitlab.com:acme/$name.git"
  done
}

# `gone <repo> <branch>` - a branch whose upstream was deleted on the forge:
# the config still points at it, the tracking ref is gone. That is what
# `%(upstream:track)` reports as [gone], and what tidy and itidy look for.
gone() {
  local repo=$1 branch=$2 name; name=$(basename "$repo")
  g -C "$repo" update-ref -d "refs/remotes/origin/$branch"
  g --git-dir="$REMOTES/$name.git" update-ref -d "refs/heads/$branch"
}

newrepo() {
  local repo="$WORK/$1"
  mkdir -p "$repo"
  g init -q "$repo"
  echo "$repo"
}

# ---------------------------------------------------------------------------
# the fixtures
# ---------------------------------------------------------------------------

write_gitconfig() {
  mkdir -p "$STAGE"
  cat > "$STAGE/.gitconfig" <<'EOF'
[user]
	name = Ada Weller
	email = ada@example.com
[init]
	defaultBranch = main
[commit]
	gpgsign = false
[advice]
	detachedHead = false
EOF
}

# billing-api: the password reached main through a hotfix branch, which is the
# case that a file listing will not find and a pickaxe will.
build_billing_api() {
  local r; r=$(newrepo billing-api)
  mkdir -p "$r/config"
  cat > "$r/config/database.yml" <<'EOF'
production:
  adapter: postgresql
  host: db.example.com
  port: 5432
  database: billing
  pool: 16
  timeout: 5000
EOF
  cat > "$r/README.md" <<'EOF'
# billing-api

Invoicing, dunning and the ledger. Talks to nothing but the database.
EOF
  c "$r" 3801600 ada "Add database config"

  g -C "$r" checkout -q -b hotfix/staging-db
  sed -i "s/  pool: 16/  password: $SECRET\n  pool: 16/" "$r/config/database.yml"
  c "$r" 3715200 marek "Unblock staging deploy"

  g -C "$r" checkout -q main
  env $(stage_env) \
      GIT_AUTHOR_NAME="Marek Toth" GIT_AUTHOR_EMAIL="marek@example.com" \
      GIT_AUTHOR_DATE="@$((NOW - 3628800)) +0000" \
      GIT_COMMITTER_NAME="Marek Toth" GIT_COMMITTER_EMAIL="marek@example.com" \
      GIT_COMMITTER_DATE="@$((NOW - 3628800)) +0000" \
      git -C "$r" merge -q --no-ff hotfix/staging-db -m "Merge hotfix/staging-db"

  sed -i "/password: $SECRET/d" "$r/config/database.yml"
  cat >> "$r/config/database.yml" <<'EOF'
  # read from the environment now, never from this file
EOF
  c "$r" 950400 ada "Read the database password from the environment"
  origin "$r"
}

# checkout-service: the repo the istatus shot is taken in, so its working tree
# carries one file of every kind git's two-column code can report.
build_checkout_service() {
  local r; r=$(newrepo checkout-service)
  mkdir -p "$r/checkout" "$r/config"
  cat > "$r/config/settings.py" <<'EOF'
"""Runtime settings, read once at import."""

import os

DEBUG = False
ALLOWED_HOSTS = ["checkout.example.com"]

DB_HOST = "db.example.com"
DB_PORT = 5432
DB_NAME = "checkout"

CURRENCY = "EUR"
CART_TTL_SECONDS = 1800
EOF
  cat > "$r/checkout/cart.py" <<'EOF'
"""The cart: items in, a total out."""

from decimal import Decimal

from checkout.tax import rate_for


class Cart:
    def __init__(self, region):
        self.region = region
        self.items = []

    def add(self, sku, price, quantity=1):
        self.items.append((sku, Decimal(price), quantity))

    def subtotal(self):
        return sum(price * quantity for _, price, quantity in self.items)

    def total(self):
        return self.subtotal() * (1 + rate_for(self.region))
EOF
  cat > "$r/checkout/tax.py" <<'EOF'
"""VAT rates, by region."""

from decimal import Decimal

RATES = {
    "de": Decimal("0.19"),
    "es": Decimal("0.21"),
    "fr": Decimal("0.20"),
}

DEFAULT = Decimal("0.20")


def rate_for(region):
    return RATES.get(region, DEFAULT)
EOF
  c "$r" 4406400 priya "Add base settings"

  sed -i "s/^DB_NAME = \"checkout\"/DB_NAME = \"checkout\"\nDB_PASSWORD = \"$SECRET\"/" "$r/config/settings.py"
  c "$r" 2764800 marek "Point staging at the new database"

  sed -i "/^DB_PASSWORD = \"$SECRET\"/c\\DB_PASSWORD = os.environ[\"CHECKOUT_DB_PASSWORD\"]" "$r/config/settings.py"
  c "$r" 1900800 ada "Read the database password from the environment"
  origin "$r"

  # A working tree with one of each: MM staged and modified again, M staged,
  # ' M' unstaged, ?? untracked. This is the picture the README explains.
  sed -i 's/^CART_TTL_SECONDS = 1800/CART_TTL_SECONDS = 3600/' "$r/config/settings.py"
  g -C "$r" add config/settings.py
  sed -i 's/^CURRENCY = "EUR"/CURRENCY = os.environ.get("CHECKOUT_CURRENCY", "EUR")/' "$r/config/settings.py"

  sed -i 's/    def add(self, sku, price, quantity=1):/    def add(self, sku, price, quantity=1):\n        if quantity < 1:\n            raise ValueError("quantity must be positive")/' "$r/checkout/cart.py"
  g -C "$r" add checkout/cart.py

  sed -i 's/^    "fr": Decimal("0.20"),/    "fr": Decimal("0.20"),\n    "it": Decimal("0.22"),/' "$r/checkout/tax.py"

  cat > "$r/notes.todo" <<'EOF'
- rounding on multi-currency carts
- cache the rate table
EOF
}

# notifications-worker: the .env mistake. Committed, then untracked, so the
# secret survives only in history - and three commits nobody has pushed.
build_notifications_worker() {
  local r; r=$(newrepo notifications-worker)
  cat > "$r/worker.py" <<'EOF'
"""Drains the notification queue and hands each message to a channel."""

import os
import time

from channels import email, push, sms

CHANNELS = {"email": email, "push": push, "sms": sms}
POLL_SECONDS = 2


def run(queue):
    while True:
        message = queue.pop()
        if message is None:
            time.sleep(POLL_SECONDS)
            continue
        CHANNELS[message.channel].send(message)
EOF
  c "$r" 5011200 priya "Add worker config"

  cat > "$r/.env" <<EOF
SMTP_HOST=smtp.example.com
SMTP_USER=notifications@example.com
SMTP_PASSWORD=$SECRET
QUEUE_URL=amqp://queue.example.com:5672
EOF
  c "$r" 4924800 priya "Add deploy environment file"

  rm "$r/.env"
  printf '.env\n__pycache__/\n' > "$r/.gitignore"
  c "$r" 4838400 ada "Stop tracking .env"
  origin "$r"

  # Three commits that exist here and on no remote: the ↑3 in the dashboard.
  printf 'RETRY_BACKOFF_SECONDS=30\n' >> "$r/.env"
  cat > "$r/retry.py" <<'EOF'
"""Exponential backoff, capped, with a little jitter."""

import random

BASE_SECONDS = 2
CAP_SECONDS = 300


def delay_for(attempt):
    window = min(CAP_SECONDS, BASE_SECONDS * 2 ** attempt)
    return random.uniform(0, window)
EOF
  c "$r" 259200 ada "Back off between delivery retries"
  sed -i 's/^POLL_SECONDS = 2/POLL_SECONDS = 1/' "$r/worker.py"
  c "$r" 172800 ada "Poll the queue twice as often"
  cat >> "$r/worker.py" <<'EOF'


def drain(queue, limit):
    """Deliver at most `limit` messages, then return."""
    for _ in range(limit):
        message = queue.pop()
        if message is None:
            return
        CHANNELS[message.channel].send(message)
EOF
  c "$r" 7200 ada "Add a bounded drain for the test harness"
}

# edge-proxy: the deep one. Enough history for the log to look real, a branch of
# every push state for ibranch and itidy, and the token-bucket rewrite that the
# diff shot is taken on.
build_edge_proxy() {
  local r; r=$(newrepo edge-proxy)
  mkdir -p "$r/src"
  cat > "$r/Cargo.toml" <<'EOF'
[package]
name = "edge-proxy"
version = "2.0.4"
edition = "2021"

[dependencies]
hyper = "1"
tokio = { version = "1", features = ["full"] }
EOF
  cat > "$r/src/main.rs" <<'EOF'
mod limiter;
mod upstream;

use std::net::SocketAddr;

fn main() {
    let addr: SocketAddr = "0.0.0.0:8080".parse().expect("valid listen address");
    println!("edge-proxy listening on {addr}");
    upstream::serve(addr);
}
EOF
  cat > "$r/src/limiter.rs" <<'EOF'
//! Per-client request limiting.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct Limiter {
    seen: HashMap<String, Vec<Instant>>,
    window: Duration,
    max: usize,
}

impl Limiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            seen: HashMap::new(),
            window,
            max,
        }
    }

    pub fn allow(&mut self, client: &str) -> bool {
        let now = Instant::now();
        let hits = self.seen.entry(client.to_string()).or_default();
        hits.retain(|t| now.duration_since(*t) < self.window);
        if hits.len() >= self.max {
            return false;
        }
        hits.push(now);
        true
    }
}
EOF
  cat > "$r/src/upstream.rs" <<'EOF'
//! Picking an upstream and forwarding to it.

use std::net::SocketAddr;

pub const UPSTREAMS: [&str; 3] = [
    "10.0.0.11:8080",
    "10.0.0.12:8080",
    "10.0.0.13:8080",
];

pub fn serve(addr: SocketAddr) {
    let _ = addr;
    todo!("wire up hyper")
}
EOF
  c "$r" 15552000 marek "Initial proxy skeleton"
  sed -i 's/todo!("wire up hyper")/unimplemented!("wire up hyper")/' "$r/src/upstream.rs"
  c "$r" 13996800 marek "Serve requests over hyper"
  printf '\n[profile.release]\nlto = true\n' >> "$r/Cargo.toml"
  c "$r" 12096000 ada "Turn on LTO for release builds"
  cat >> "$r/src/upstream.rs" <<'EOF'

/// Round-robin, because the upstreams are identical.
pub fn pick(counter: usize) -> &'static str {
    UPSTREAMS[counter % UPSTREAMS.len()]
}
EOF
  c "$r" 10368000 priya "Round-robin between upstreams"
  cat > "$r/src/health.rs" <<'EOF'
//! Liveness for the load balancer in front of us.

pub fn healthy() -> bool {
    true
}
EOF
  sed -i 's/^mod limiter;/mod health;\nmod limiter;/' "$r/src/main.rs"
  c "$r" 8640000 priya "Add a health endpoint"
  sed -i 's/version = "2.0.4"/version = "2.1.0"/' "$r/Cargo.toml"
  c "$r" 6912000 ada "Release 2.1.0"

  # The star of the diff shot: a whole strategy swapped out, so the pane shows
  # removals on the left and their replacement on the right.
  cat > "$r/src/limiter.rs" <<'EOF'
//! Per-client request limiting, as a leaky bucket.
//!
//! The previous window kept every timestamp it had seen, so a client that
//! hammered us cost memory proportional to how hard it hammered. A bucket is
//! two numbers per client and cannot grow.

use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Bucket {
    level: f64,
    drained: Instant,
}

pub struct Limiter {
    buckets: HashMap<String, Bucket>,
    capacity: f64,
    drain_per_second: f64,
}

impl Limiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            buckets: HashMap::new(),
            capacity: max as f64,
            drain_per_second: max as f64 / window.as_secs_f64(),
        }
    }

    pub fn allow(&mut self, client: &str) -> bool {
        let now = Instant::now();
        let capacity = self.capacity;
        let drain = self.drain_per_second;
        let bucket = self.buckets.entry(client.to_string()).or_insert(Bucket {
            level: 0.0,
            drained: now,
        });

        let elapsed = now.duration_since(bucket.drained).as_secs_f64();
        bucket.level = (bucket.level - elapsed * drain).max(0.0);
        bucket.drained = now;

        if bucket.level + 1.0 > capacity {
            return false;
        }
        bucket.level += 1.0;
        true
    }

    /// Drop clients whose bucket has had time to empty completely.
    pub fn evict_idle(&mut self, idle: Duration) {
        let now = Instant::now();
        self.buckets
            .retain(|_, b| now.duration_since(b.drained) < idle);
    }
}
EOF
  c "$r" 5184000 marek "Rewrite the limiter around a leaky bucket"

  cat > "$r/src/upstream.rs" <<'EOF'
//! Picking an upstream and forwarding to it.

use std::net::SocketAddr;
use std::time::Duration;

pub const UPSTREAMS: [&str; 3] = [
    "10.0.0.11:8080",
    "10.0.0.12:8080",
    "10.0.0.13:8080",
];

/// How long a shutdown waits for in-flight requests before it stops waiting.
pub const DRAIN: Duration = Duration::from_secs(30);

pub fn serve(addr: SocketAddr) {
    let _ = addr;
    unimplemented!("wire up hyper")
}

/// Round-robin, because the upstreams are identical.
pub fn pick(counter: usize) -> &'static str {
    UPSTREAMS[counter % UPSTREAMS.len()]
}
EOF
  c "$r" 3456000 priya "Drain in-flight requests before shutdown"
  sed -i 's/^pub fn healthy() -> bool {/pub fn healthy() -> bool {\n    \/\/ TODO: report upstream reachability, not just our own process\n/' "$r/src/health.rs"
  c "$r" 1728000 ada "Note what the health endpoint still does not check"
  origin "$r"

  # release/v2.1: pushed and left alone, so ibranch has something reading
  # "synced" next to all the drama.
  g -C "$r" checkout -q -b release/v2.1
  g -C "$r" push -q -u origin release/v2.1

  # Two branches whose upstream was deleted after the merge: itidy's whole list.
  g -C "$r" checkout -q -b fix/upstream-timeout main
  sed -i 's/Duration::from_secs(30)/Duration::from_secs(15)/' "$r/src/upstream.rs"
  c "$r" 1209600 marek "Halve the drain timeout"
  g -C "$r" push -q -u origin fix/upstream-timeout
  gone "$r" fix/upstream-timeout

  g -C "$r" checkout -q -b chore/bump-deps main
  sed -i 's/^hyper = "1"/hyper = "1.4"/' "$r/Cargo.toml"
  c "$r" 864000 priya "Bump hyper to 1.4"
  g -C "$r" push -q -u origin chore/bump-deps
  gone "$r" chore/bump-deps

  g -C "$r" checkout -q -b feat/health-detail main
  sed -i 's|    // TODO: report upstream reachability, not just our own process|    // Reports our own process only; upstream reachability is the next one.|' "$r/src/health.rs"
  c "$r" 1555200 ada "Say what the health endpoint covers"
  g -C "$r" push -q -u origin feat/health-detail
  gone "$r" feat/health-detail

  g -C "$r" checkout -q -b fix/round-robin-overflow main
  sed -i 's|    UPSTREAMS\[counter % UPSTREAMS.len()\]|    UPSTREAMS[counter.wrapping_rem(UPSTREAMS.len())]|' "$r/src/upstream.rs"
  c "$r" 2073600 marek "Stop the round-robin counter overflowing"
  g -C "$r" push -q -u origin fix/round-robin-overflow
  gone "$r" fix/round-robin-overflow

  # Pushed, then carried on: ↑1.
  g -C "$r" checkout -q -b feat/rate-limiter main
  g -C "$r" push -q -u origin feat/rate-limiter
  cat >> "$r/src/limiter.rs" <<'EOF'

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_client_gets_a_whole_empty_bucket() {
        let mut l = Limiter::new(3, Duration::from_secs(1));
        assert!(l.allow("198.51.100.7"));
        assert!(l.allow("198.51.100.7"));
        assert!(l.allow("198.51.100.7"));
        assert!(!l.allow("198.51.100.7"));
    }
}
EOF
  c "$r" 432000 marek "Test the bucket refill"

  # Never pushed anywhere: "no remote".
  g -C "$r" checkout -q -b spike/http3 main
  cat > "$r/src/quic.rs" <<'EOF'
//! A spike. Do not merge: no congestion control, no retries, no tests.

pub fn negotiate() -> Option<&'static str> {
    None
}
EOF
  c "$r" 300000 marek "Spike: terminate HTTP/3 at the edge"

  g -C "$r" checkout -q main
  # Two commits that only the remote has, so main reads ↓2: push them, then
  # rewind the local branch and leave the tracking ref where it was.
  sed -i 's/^tokio = /tokio = /' "$r/Cargo.toml"
  printf '\n# vendored while the upstream fix lands\n' >> "$r/Cargo.toml"
  c "$r" 180000 priya "Vendor the hyper fix"
  sed -i 's/version = "2.1.0"/version = "2.1.1"/' "$r/Cargo.toml"
  c "$r" 90000 priya "Release 2.1.1"
  g -C "$r" push -q origin main
  g -C "$r" reset -q --hard HEAD~2
}

# design-system: clean and synced. A list where everything needs attention
# teaches less than one where two things do not.
build_design_system() {
  local r; r=$(newrepo design-system)
  cat > "$r/palette.json" <<'EOF'
{
  "color": {
    "surface": "#11111b",
    "text": "#cdd6f4",
    "accent": "#89b4fa",
    "danger": "#f38ba8"
  },
  "radius": { "sm": 2, "md": 6, "lg": 12 },
  "space": [0, 4, 8, 12, 16, 24, 32]
}
EOF
  c "$r" 7776000 priya "The first pass at a palette"
  sed -i 's/"accent": "#89b4fa"/"accent": "#89b4fa",\n    "muted": "#6c7086"/' "$r/palette.json"
  c "$r" 2592000 priya "Add a muted foreground"
  origin "$r"
}

# infra-terraform: uncommitted work and one commit nobody has pushed, so the
# dashboard shows both markers on one row.
build_infra_terraform() {
  local r; r=$(newrepo infra-terraform)
  cat > "$r/main.tf" <<'EOF'
terraform {
  required_version = ">= 1.6"
}

variable "region" {
  type    = string
  default = "eu-west-1"
}

module "edge" {
  source        = "./modules/edge"
  region        = var.region
  instance_type = "t3.small"
  desired_count = 2
}
EOF
  c "$r" 6048000 marek "Stand up the edge module"
  sed -i 's/  desired_count = 2/  desired_count = 4/' "$r/main.tf"
  c "$r" 604800 marek "Scale the edge to four"
  origin "$r"
  sed -i 's/  instance_type = "t3.small"/  instance_type = "t3.medium"/' "$r/main.tf"
  c "$r" 3600 marek "Move the edge to t3.medium"
  sed -i 's/  default = "eu-west-1"/  default = "eu-central-1"/' "$r/main.tf"
}

# ---------------------------------------------------------------------------
# up / shell / down
# ---------------------------------------------------------------------------

up() {
  [ -x "$SLU" ] || { echo "no release binary at $SLU - run: cargo build --release" >&2; exit 1; }
  down_quiet
  NOW=$(date +%s)
  mkdir -p "$WORK" "$REMOTES"
  write_gitconfig
  build_billing_api
  build_checkout_service
  build_notifications_worker
  build_edge_proxy
  build_design_system
  build_infra_terraform
  seal_remotes
  echo "staged in $STAGE"
  echo
  env $(stage_env) sh -c "cd '$WORK' && '$SLU' repos"
  echo
  echo "  ./stage.sh shell   a shell where slu is this build"
  echo "  ./stage.sh down    delete it all"
}

# Nothing here mounts anything, but a recursive delete in a convenience script
# gets one of these anyway: rm walks straight through a mountpoint onto whatever
# is on the far side, and --one-file-system is what stops it.
down_quiet() {
  [ -d "$STAGE" ] || return 0
  if awk -v s="$STAGE/" '$2 ~ "^"s {found=1} END {exit !found}' /proc/mounts 2>/dev/null; then
    echo "REFUSING to delete $STAGE: something is mounted under it." >&2
    exit 1
  fi
  chmod -R u+w "$STAGE" 2>/dev/null || true
  rm -rf --one-file-system "$STAGE"
}

# A shell where `slu` is this build and `~` is the stage. It sources your real
# ~/.bashrc by absolute path (the redirect hides it) so the prompt in the shot
# is your own, then clears, because a bashrc that greets you would greet the
# README too.
open_shell() {
  mkdir -p "$HERE/bin" "$STAGE/.local"
  ln -sf "$(cd "$(dirname "$SLU")" && pwd)/slu" "$HERE/bin/slu"
  # A prompt that shells out to a helper of yours (starship's `custom` modules
  # do) would find nothing under the staged HOME and quietly render an empty
  # segment. A symlink to the real ~/.local/bin is enough; `rm` unlinks a
  # symlink rather than walking into it, so teardown still cannot reach it.
  [ -d "$HOME/.local/bin" ] && ln -sfn "$HOME/.local/bin" "$STAGE/.local/bin"
  {
    echo "[ -f '$HOME/.bashrc' ] && . '$HOME/.bashrc'"
    echo "clear"
  } > "$HERE/shellrc"
  (cd "$WORK" && env $(stage_env) \
    PATH="$HERE/bin:$PATH" \
    STARSHIP_CONFIG="$HOME/.config/starship.toml" \
    bash --noprofile --rcfile "$HERE/shellrc" -i)
}

case "${1:-up}" in
  up)    up ;;
  shell) open_shell ;;
  down)  down_quiet; echo "torn down" ;;
  *)     echo "usage: $0 [up|shell|down]" >&2; exit 2 ;;
esac
