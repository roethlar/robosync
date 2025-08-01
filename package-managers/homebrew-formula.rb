class Robosync < Formula
  desc "High-performance file synchronization with intelligent concurrent processing"
  homepage "https://github.com/roethlar/robosync"
  version "1.0.3"
  license "MIT"

  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/roethlar/robosync/releases/download/v1.0.3/robosync-1.0.3-aarch64-apple-darwin.tar.gz"
      sha256 "AARCH64_DARWIN_SHA256_PENDING"
    else
      url "https://github.com/roethlar/robosync/releases/download/v1.0.3/robosync-1.0.3-x86_64-apple-darwin.tar.gz"
      sha256 "8f9a471058649a765011ff27028aab8bce13b63bd7d33f0c1a41e6092991ce1e"
    end
  elsif OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/roethlar/robosync/releases/download/v1.0.3/robosync-1.0.3-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "AARCH64_LINUX_SHA256_PENDING"
    else
      url "https://github.com/roethlar/robosync/releases/download/v1.0.3/robosync-1.0.3-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "9f634703388d85c443bfbf56cf489daa9f93129c8bf9fdd1867f83c2c5fb467c"
    end
  end

  def install
    bin.install "robosync"
  end

  test do
    system "#{bin}/robosync", "--version"
  end
end