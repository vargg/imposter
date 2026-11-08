# Running checks in CI

`imposter run` can check an API in CI with the same requests you edit in the app. CI has no
copy of your I’mposter data, so it runs from an exported collection file.

1. In the app, open the collection's **⋯** menu → **Export Collection…** and tick the
   environments the run needs. Commit the `.imposter.json` next to your code. Environments
   in the export hold their variables' values, so keep secrets out of them: leave a secret
   variable empty or undefined and pass it with `--secret` instead.
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
          imposter run "Shop API / Smoke" --file api.imposter.json --env CI
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
