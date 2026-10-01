// The local course engine uses standard Web APIs, independently of Next.js.
export class NextRequest extends Request {
  get nextUrl() {
    return new URL(this.url);
  }
}
export const NextResponse = Response;
