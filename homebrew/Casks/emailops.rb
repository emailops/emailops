cask "emailops" do
  version "0.6.11"
  sha256 "a585d196df02687811ef5113b75ac2428deba890cce8b2af6ad3b0e32b33f022"

  url "https://github.com/emailops/emailops/releases/download/v#{version}/EmailOps-macos.dmg"
  name "EmailOps"
  desc "Privacy-first, AI-native email client"
  homepage "https://github.com/emailops/emailops"

  livecheck do
    url :url
    strategy :github_latest
  end

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
