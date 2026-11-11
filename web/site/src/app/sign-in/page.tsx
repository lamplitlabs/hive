import type { Metadata } from "next";
import { Door } from "@/components/door";

export const metadata: Metadata = {
  title: "Sign in",
  description: "Sign in to your Hive team: one login for every machine your team runs Hive on.",
  robots: { index: false },
};

export default function SignInPage() {
  return <Door kind="sign-in" />;
}
