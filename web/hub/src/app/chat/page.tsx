"use client";

import { Suspense } from "react";
import { ChatPageClient } from "@/components/chat/chat";
import { SkeletonPage } from "@/components/hive/panel";

export default function ChatPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <ChatPageClient />
    </Suspense>
  );
}
