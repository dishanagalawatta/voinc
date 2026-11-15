cask "voinc" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "@URL@"
  name "Voinc"
  desc "Local voice dictation"
  homepage "https://anoma.ly/hex/"

  livecheck do
    url "https://anoma.ly/release/hex/desktop/stable.json"
    strategy :json do |json|
      json.dig("release", "version")
    end
  end

  auto_updates true
  depends_on arch: :arm64
  depends_on macos: :sequoia

  app "Voinc.app"

  uninstall quit: "io.voinc.voinc"

  zap trash: [
    "~/.config/voinc",
    "~/Library/Application Support/voinc",
    "~/Library/Caches/io.voinc.voinc",
    "~/Library/HTTPStorages/io.voinc.voinc",
    "~/Library/Preferences/io.voinc.voinc.plist",
    "~/Library/Saved Application State/io.voinc.voinc.savedState",
  ]
end
