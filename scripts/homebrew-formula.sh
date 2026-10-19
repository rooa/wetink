#!/usr/bin/env bash
# Print the Homebrew formula for a release.
#   scripts/homebrew-formula.sh VERSION DIST_DIR
# DIST_DIR holds the release archives and their .sha256 files.
set -euo pipefail
version=$1
dist=$2
repo=${GITHUB_REPOSITORY:-rooa/wetink}

asset() {
  local name="wetink-$version-$1.tar.gz"
  local sum
  sum=$(cut -d' ' -f1 "$dist/$name.sha256")
  printf '      url "https://github.com/%s/releases/download/v%s/%s"\n' "$repo" "$version" "$name"
  printf '      sha256 "%s"\n' "$sum"
}

cat <<RUBY
class Wetink < Formula
  desc "Live Markdown viewer for the terminal that highlights changed blocks"
  homepage "https://github.com/$repo"
  version "$version"
  license "MIT"

  on_macos do
    on_arm do
$(asset aarch64-apple-darwin)
    end
    on_intel do
$(asset x86_64-apple-darwin)
    end
  end

  on_linux do
    on_arm do
$(asset aarch64-unknown-linux-musl)
    end
    on_intel do
$(asset x86_64-unknown-linux-musl)
    end
  end

  def install
    bin.install "wetink"
  end

  test do
    (testpath/"t.md").write "# Hello\n"
    assert_match "Hello", shell_output("#{bin}/wetink --print #{testpath}/t.md")
  end
end
RUBY
