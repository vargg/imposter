# Running checks in CI

`imposter run` can check an API in CI with the same requests you edit in the app. Your
collections are files, so CI runs them straight from the repository: nothing to export.

1. In the app, set **Settings → Collections folder** to a folder in your repository (for
   example `api/`) and commit it. Each request is its own YAML file, so an edit is a small
   diff. Values your environments share are in their files; keep secrets out of them:
   write `{{secret:token}}` in requests and keep the value in the environment's secrets,
   which never leave your computer. In CI, pass it with `--secret token=API_TOKEN`.
2. Add a job that installs I’mposter and runs the collection, a folder or one request:

```yaml
# .github/workflows/api.yml
name: API checks
on: [push]
jobs:
  api:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install I’mposter
        run: |
          gh release download --repo vargg/imposter --pattern '*.deb' --dir /tmp/imposter
          sudo apt-get install -y /tmp/imposter/*.deb
        env:
          GH_TOKEN: ${{ github.token }}
      - name: Run the checks
        run: >
          imposter run "Shop API / Smoke" --project api --env CI
          --secret token=API_TOKEN --junit report.xml --out report.json
        env:
          API_TOKEN: ${{ secrets.API_TOKEN }}
      - name: Keep the report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: api-report
          path: |
            report.xml
            report.json
```

The job fails when the run does not pass. Each outcome has its own exit status
(`imposter --help` lists them), so a step can tell a failed check (1) from a server that
was not there (4). `report.xml` is JUnit XML, which most CI systems show as a test report;
`report.json` is the full report, with each request's result.

Secrets given with `--secret`, and the credentials requests send, are replaced by
`«redacted»` in the report, the JUnit file and the log.

Without a collections folder in the repository, export the collection instead
(**⋯ → Export Collection…**) and give the file with `--file api.imposter.json`.
