cask "emailops" do
  version "0.6.3"
  sha256 "dfaf8223d1b67a513c93235b191ff0ff7cc0d290014cd9f921912b9f64cab470"

  url "https://github.com/emailops/emailops/releases/download/v#{version}/EmailOps-macos.dmg"
  name "EmailOps"
  desc "Privacy-first, AI-native email client"
  homepage "https://github.com/emailops/emailops"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on arch: :arm64
  depends_on macos: ">= :monterey"

  app "EmailOps.app"

  zap trash: [
    "~/Library/Application Support/com.emailops.app",
    "~/Library/Caches/com.emailops.app",
    "~/Library/HTTPStorages/com.emailops.app",
    "~/Library/Preferences/com.emailops.app.plist",
    "~/Library/Saved Application State/com.emailops.app.savedState",
    "~/Library/WebKit/com.emailops.app",
  ]
end
