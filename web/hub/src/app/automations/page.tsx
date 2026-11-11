"use client";

import { Suspense } from "react";
import { AutomationsPageClient } from "@/components/automations/automations";
import { SkeletonPage } from "@/components/hive/panel";

export default function AutomationsPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <AutomationsPageClient />
    </Suspense>
  );
}
