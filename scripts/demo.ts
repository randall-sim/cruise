import { seedDemo } from "../src/lib/demo";
import nextEnv from "@next/env";
nextEnv.loadEnvConfig(process.cwd());
await seedDemo();
console.log(
  "Created three clearly labeled demo courses in the private workspace.",
);
