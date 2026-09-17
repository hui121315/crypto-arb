const qaProfile = process.env.CROSSLINE_E2E_PROFILE ?? "dev";
const frontendCommand = qaProfile === "release"
  ? `python3 -m http.server ${webPort}`
  : `cd frontend && env -u NO_COLOR trunk build --release=false && python3 -m http.server ${webPort} --bind 127.0.0.1 --directory dist`;
