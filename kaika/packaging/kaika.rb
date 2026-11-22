# The formula for the tap (Formula/kaika.rb). See README.md in this folder for how a release is cut.
# The URL exists once the v0.1.0 release has been published; the sha256 is that of the tarball attached to it
# (README.md, step 3, replaces it with the published value).
class Kaika < Formula
  desc "Universal package manager"
  homepage "https://github.com/yozarii/YZKR-Yozakura"
  url "https://github.com/yozarii/YZKR-Yozakura/releases/download/v0.1.0/kaika-0.1.0.tar.gz"
  sha256 "177d6c56d95bf57089f48b9d785cf7e7b5e23a93f577731f4620b8a125673922"
  license "MIT"

  depends_on "rust" => :build
  depends_on arch: :arm64
  depends_on :macos

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "kaika #{version}", shell_output("#{bin}/kaika --version")
    assert_match "kaika   #{version}", shell_output("#{bin}/kaika doctor")
  end
end
