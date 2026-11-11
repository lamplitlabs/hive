import type { Metadata } from "next";
import { Door } from "@/components/door";

export const metadata: Metadata = {
  title: "Create your account",
  description: "Create the Hive account every machine in your team inherits.",
  robots: { index: false },
};

export default function SignUpPage() {
  return <Door kind="sign-up" />;
}
