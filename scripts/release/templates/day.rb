# day.rb — Homebrew formula for the `day` CLI (macOS and Linux).
#
# Rendered from scripts/release/templates/day.rb by render-installers.py for release
# __DAY_VERSION__ (per-platform URLs and sha256 checksums baked in, in the style of cargo-dist's
# Homebrew installer). Install directly from the release asset:
#
#   curl -LO __DAY_INSTALLER_BASE__/day.rb && brew install --formula ./day.rb
#
# (The same file is ready to publish into a Homebrew tap unchanged.)
class Day < Formula
  desc "Cross-platform native apps in Rust — build, run, test, and package for every platform"
  homepage "https://daybrite.dev"
  version "__DAY_VERSION__"

  on_macos do
    if Hardware::CPU.arm?
      url "__DAY_BASE_URL__/day-aarch64-apple-darwin.tar.gz"
      sha256 "__SHA256_aarch64_apple_darwin__"
    else
      url "__DAY_BASE_URL__/day-x86_64-apple-darwin.tar.gz"
      sha256 "__SHA256_x86_64_apple_darwin__"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "__DAY_BASE_URL__/day-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_aarch64_unknown_linux_gnu__"
    else
      url "__DAY_BASE_URL__/day-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "__SHA256_x86_64_unknown_linux_gnu__"
    end
  end

  def install
    bin.install "day"
  end

  test do
    system "#{bin}/day", "--version"
  end
end
