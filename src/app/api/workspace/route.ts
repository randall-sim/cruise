import { NextRequest, NextResponse } from "@/lib/web-request";
import { readState } from "@/lib/store";
import { localRequest, errorResponse } from "@/lib/http";
export const dynamic = "force-dynamic";
export async function GET(request: NextRequest) {
  try {
    localRequest(request);
    const {
      tasks: _tasks,
      settings: _settings,
      sync: _sync,
      ...state
    } = await readState();
    return NextResponse.json(
      {
        ...state,
        // Hiding affects the dashboard only; stored evidence and agent memory remain intact.
        lectures: state.lectures.filter((lecture) => !lecture.hiddenFromUi),
        jobs: state.jobs.map(({ context, ...job }) => ({
          ...job,
          context: context.filter((c) => job.result?.citations.includes(c.id)),
          evidenceCount: context.length,
        })),
        policy: {
          assignmentDrafts: true,
          submissions: false,
          downloads: true,
          execution: true,
        },
      },
      { headers: { "Cache-Control": "no-store" } },
    );
  } catch (error) {
    return errorResponse(error);
  }
}
