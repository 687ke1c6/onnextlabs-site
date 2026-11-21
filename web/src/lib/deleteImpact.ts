/** What a delete would do, from the server's walk over the database's foreign keys. */
export type DeleteImpact = {
  effects: {
    table: string;
    /** Plain words for the rows, e.g. "statement rows". */
    label: string;
    /** Deleted too / kept with their link cleared / blocking the delete. */
    action: "delete" | "unlink" | "block";
    count: number;
  }[];
  blocked: boolean;
};
