cask "emailops" do
  arch intel: "-intel"

  version "0.6.5"
  sha256 arm:   "3a905dd7d9684ef53b0039811ef30372524e5813145c5ac1ea8ec820b89f6a5f",
         intel: "acf6bf39dc1e06742d6d3c577e011a544c47ae0cc8fac17d1131686c931547a3"

  url "https://github.com/emailops/emailops/releases/download/v#{version}/EmailOps-macos#{arch}.dmg"
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
