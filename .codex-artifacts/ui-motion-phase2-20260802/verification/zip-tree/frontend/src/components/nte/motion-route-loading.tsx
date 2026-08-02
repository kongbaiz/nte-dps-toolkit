import { Skeleton } from "@/components/ui/skeleton";
import { t } from "@/lib/i18n";

export function MotionRouteLoading({ compact = false }: { compact?: boolean }) {
  if (compact) {
    return (
      <main
        className="motion-route-loading grid h-screen place-items-center bg-transparent"
        aria-busy="true"
        aria-label={t("Loading")}
      >
        <Skeleton className="h-[calc(100%-0.75rem)] w-[calc(100%-0.75rem)] rounded-2xl" />
      </main>
    );
  }
  return (
    <main
      className="motion-route-loading grid h-screen grid-rows-[2rem_1fr] bg-background p-3"
      aria-busy="true"
      aria-label={t("Loading")}
    >
      <Skeleton className="h-5 w-36" />
      <div className="grid min-h-0 grid-cols-[12rem_1fr] gap-3 pt-4">
        <Skeleton className="h-full min-h-40" />
        <div className="grid content-start gap-3">
          <Skeleton className="h-20" />
          <Skeleton className="h-36" />
          <Skeleton className="h-10" />
        </div>
      </div>
    </main>
  );
}
