import { resolve } from "node:path";
import { executableName } from "./platform.mjs";

export function engineProfile(
  args = process.argv.slice(2),
  env = process.env,
  fallback = process.platform === "linux" ? "debug" : "release",
) {
  const named = args
    .find((arg) => arg.startsWith("--engine-profile="))
    ?.slice("--engine-profile=".length);
  const index = args.indexOf("--engine-profile");
  if (index >= 0 && (!args[index + 1] || args[index + 1].startsWith("--")))
    throw new Error("缺少引擎构建类型。");
  const profile = args.includes("--release")
    ? "release"
    : (named ??
      (index >= 0 ? args[index + 1] : undefined) ??
      env.STUDIO_ENGINE_PROFILE ??
      fallback);
  if (!["release", "debug"].includes(profile))
    throw new Error("引擎构建类型必须是 release 或 debug。");
  return profile;
}
export const engineBuildArguments = (profile) => [
  "build",
  "-p",
  "studio-engine",
  ...(profile === "release" ? ["--release"] : []),
];
export const engineExecutable = (root, profile) =>
  resolve(root, "target", profile, executableName("studio-engine"));
