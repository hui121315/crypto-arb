const qaProfile = process.env.CROSSLINE_E2E_PROFILE ?? "dev";
const trunkProfileArgs = qaProfile === "release"
  ? "--release=true --locked=true"
  : "--release=false";
