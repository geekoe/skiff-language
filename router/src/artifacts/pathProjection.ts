import {
  isPublicationId,
  publicationStorageSegment,
} from "../publicationId.js";

export function serviceIdPathSegments(serviceId: string): string[] {
  if (!isPublicationId(serviceId)) {
    throw new Error(`serviceId ${serviceId} must be a publication id`);
  }
  return [publicationStorageSegment(serviceId)];
}

export function serviceIdPath(serviceId: string): string {
  return serviceIdPathSegments(serviceId).join("/");
}
