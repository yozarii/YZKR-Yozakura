# The formula for the tap (Formula/kaika.rb). See README.md in this folder for how a release is cut.
# The URL exists once the v0.1.0 release has been published. The sha256 below is that of a tarball built locally from
# the release commit; README.md, step 3, replaces it with the checksum published on the release.
class Kaika < Formula
  desc "Universal package manager"
  homepage "https://github.com/yozarii/YZKR-Yozakura"
  url "https://github.com/yozarii/YZKR-Yozakura/releases/download/v0.1.0/kaika-0.1.0.tar.gz"
  sha256 "9c987e5832f606b0239a06b72d67eef71def55d8a70d1a2f6e967fbdd829ae1d"
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
