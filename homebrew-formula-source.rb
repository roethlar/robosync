class Robosync < Formula
  desc "High-performance file synchronization with intelligent concurrent processing"
  homepage "https://github.com/roethlar/robosync"
  url "https://github.com/roethlar/robosync/archive/refs/tags/v1.0.1.tar.gz"
  sha256 "4a49d80e51eab73e7a26360a207a412e8ff930dd6fb0967322b2f2718ed3f956"
  license "MIT"
  head "https://github.com/roethlar/robosync.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    # Create test directories
    (testpath/"source").mkpath
    (testpath/"source/test.txt").write("Hello, RoboSync!")
    
    # Run robosync
    system bin/"robosync", testpath/"source", testpath/"dest"
    
    # Verify the file was copied
    assert_predicate testpath/"dest/test.txt", :exist?
    assert_equal "Hello, RoboSync!", (testpath/"dest/test.txt").read
  end
end