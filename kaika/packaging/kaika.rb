# The formula for the tap (Formula/kaika.rb). See README.md in this folder for how a release is cut.
# The URL exists once the v0.1.0 release has been published; the sha256 is that of the tarball attached to it
# (README.md, step 3, replaces it with the published value).
class Kaika < Formula
  desc "Universal package manager"
  homepage "https://github.com/yozarii/YZKR-Yozakura"
  url "https://github.com/yozarii/YZKR-Yozakura/releases/download/v0.1.0/kaika-0.1.0.tar.gz"
  sha256 "960848af14b8bfb614d55c5c625c87e376cb6f02f3e6af6616bcf944193bf282"
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
