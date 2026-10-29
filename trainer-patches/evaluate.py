import argparse
import hashlib
import json
import os

import features
import model as M
import nnue_dataset
import torch
from torch.utils.data import DataLoader


def validation_loader(filename, feature_set, batch_size, validation_size, device):
  if not os.path.exists(filename):
    raise Exception('{} does not exist'.format(filename))
  dataset = nnue_dataset.SparseBatchDataset(
      feature_set.name, filename, batch_size, filtered=False,
      random_fen_skipping=0, device=device)
  batches = (validation_size + batch_size - 1) // batch_size
  return DataLoader(nnue_dataset.FixedNumBatchesDataset(dataset, batches),
                    batch_size=None, batch_sampler=None)


def loss_for_file(nnue, filename, feature_set, batch_size, validation_size, device):
  loader = validation_loader(filename, feature_set, batch_size, validation_size, device)
  total = 0.0
  count = 0
  with torch.no_grad():
    for batch in loader:
      loss = nnue.loss_value(batch)
      current = batch[0].shape[0]
      total += float(loss.detach().cpu()) * current
      count += current
  if count == 0:
    raise Exception('{} produced no validation positions'.format(filename))
  return total / count, count


def sha256(filename):
  digest = hashlib.sha256()
  with open(filename, 'rb') as stream:
    for chunk in iter(lambda: stream.read(1024 * 1024), b''):
      digest.update(chunk)
  return digest.hexdigest()


def main():
  parser = argparse.ArgumentParser(
      description='Deterministically evaluate a checkpoint on ID and OOD binaries.')
  parser.add_argument('checkpoint')
  parser.add_argument('--id-validation', required=True)
  parser.add_argument('--ood-validation', required=True)
  parser.add_argument('--features', required=True)
  parser.add_argument('--batch-size', type=int, default=16384)
  parser.add_argument('--validation-size', type=int, default=100000)
  parser.add_argument('--output', required=True)
  args = parser.parse_args()

  if args.batch_size <= 0 or args.validation_size <= 0:
    raise Exception('--batch-size and --validation-size must be positive')
  device = 'cuda' if torch.cuda.is_available() else 'cpu'
  feature_set = features.get_feature_set_from_name(args.features)
  nnue = M.NNUE.load_from_checkpoint(args.checkpoint, feature_set=feature_set)
  nnue.eval()
  nnue.to(device)
  id_loss, id_positions = loss_for_file(
      nnue, args.id_validation, feature_set, args.batch_size, args.validation_size, device)
  ood_loss, ood_positions = loss_for_file(
      nnue, args.ood_validation, feature_set, args.batch_size, args.validation_size, device)
  result = {
      'schema': 'haitaka-nnue-offline-evaluation-v1',
      'checkpoint': os.path.abspath(args.checkpoint),
      'checkpoint_sha256': sha256(args.checkpoint),
      'features': feature_set.name,
      'deterministic': True,
      'random_fen_skipping': 0,
      'id_validation': {
          'path': os.path.abspath(args.id_validation),
          'positions': id_positions,
          'loss': id_loss,
      },
      'legacy_ood_validation': {
          'path': os.path.abspath(args.ood_validation),
          'positions': ood_positions,
          'loss': ood_loss,
          'opening_scope': 'two-opening OOD diagnostic',
      },
  }
  with open(args.output, 'w', encoding='utf-8') as stream:
    json.dump(result, stream, indent=2, sort_keys=True)
    stream.write('\n')


if __name__ == '__main__':
  main()
